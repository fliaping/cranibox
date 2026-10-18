//! The current entry coordinates a reviewed snapshot, not a live session migration.
use super::*;
use crate::{
    config as secrets,
    projects::copy::{self, Package},
    projects::handoff::Handoff,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    sync::{Arc, OnceLock},
};
mod streaming;
const MAX_JOB: usize = copy::stream::MAX_MANIFEST + 65536;

// Local ownership is authenticated by the hosting CLI, Web or native IPC. A
// node cannot grant its own ID over HTTP, so these receipts cannot be queried
// using a remote grant. This marker is not a network credential.
const LOCAL_OWNER: &str = "cranibox-local-project-copy-v1";
static LOCAL_SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
fn local_slot() -> Result<tokio::sync::OwnedSemaphorePermit> {
    LOCAL_SLOTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(crate::protocol::MAX_PARALLEL)))
        .clone()
        .try_acquire_owned()
        .context("local copy capacity reached")
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    protocol: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    progress: Option<Value>,
    id: String,
    source: String,
    target: String,
    replica_id: String,
    binding: String,
    digest: String,
    target_root: PathBuf,
    state: String,
    created_at: u64,
    summary: Value,
    result: Option<Value>,
    error: Option<String>,
}
impl Job {
    fn bytes(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)?;
        ensure!(
            bytes.len() <= MAX_JOB,
            "copy manifest exceeds its metadata limit"
        );
        Ok(bytes)
    }
    fn view(&self) -> Value {
        json!({"id":self.id,"protocol":self.protocol.unwrap_or(1),"progress":self.progress,"source":self.source,"target":self.target,"replica_id":self.replica_id,"digest":self.digest,"target_root":self.target_root,"state":self.state,"created_at":self.created_at,"summary":self.summary,"result":self.result,"error":self.error})
    }
}
impl Store {
    fn copy_directory(&self) -> PathBuf {
        self.path.with_file_name("project-copy-jobs")
    }
    fn copy_path(&self, id: &str) -> Result<PathBuf> {
        Ok(self
            .copy_directory()
            .join(format!("{}.json", uuid::Uuid::parse_str(id)?)))
    }
    fn copy_binding(&self, source: &str, target: &str) -> Result<String> {
        let config = Config::read(&self.path)?;
        let mut values = vec![json!(config.node_id)];
        for node in [source, target] {
            ensure!(valid_node_id(node), "invalid copy target");
            if node == config.node_id {
                values.push(json!(["local", node, self.local_projects.path()]));
                continue;
            }
            let route = config.routes.get(node).context("copy connection removed")?;
            values.push(json!([
                node,
                route.url,
                route.allow_insecure,
                secrets::read_token(&route.token_file)?
            ]));
        }
        Ok(copy::digest(&serde_json::to_vec(&values)?))
    }
    fn local_copy_permitted(&self, node: &str, root: Option<&Path>) -> Result<()> {
        let config = Config::read(&self.path)?;
        ensure!(config.node_id == node, "local copy identity changed");
        if let Some(root) = root {
            ensure!(
                config.copy_root.as_deref() == Some(root),
                "project copy root changed or disabled"
            );
        }
        Ok(())
    }
    fn read_copy(&self, id: &str) -> Result<Job> {
        let file = fs::File::open(self.copy_path(id)?).context("copy record unavailable")?;
        ensure!(
            file.metadata()?.len() <= MAX_JOB as u64,
            "copy record too large"
        );
        let mut job: Job = serde_json::from_reader(file)?;
        ensure!(
            job.id == id && job.binding == self.copy_binding(&job.source, &job.target)?,
            "copy connections or credentials changed; prepare a new snapshot"
        );
        if job.protocol == Some(2)
            && matches!(
                job.state.as_str(),
                "preparing"
                    | "prepare_paused"
                    | "sending"
                    | "uncertain"
                    | "receiving"
                    | "publishing"
            )
        {
            let path = self.copy_path(id)?.with_extension("progress");
            if let Ok(file) = fs::File::open(path)
                && file.metadata()?.len() < 16384
            {
                job.progress = serde_json::from_reader(file).ok();
            }
        }
        Ok(job)
    }
    fn save_copy(&self, job: &Job) -> Result<()> {
        secrets::write_private(&self.copy_path(&job.id)?, &job.bytes()?, true)
    }
    fn copy_package_path(&self, id: &str) -> Result<PathBuf> {
        Ok(self.copy_path(id)?.with_extension("package"))
    }
    pub(super) fn copy_list(&self) -> Result<Value> {
        self.copy_history(false)
    }
    pub(super) fn copy_activity(&self) -> Result<Value> {
        self.copy_history(true)
    }
    fn copy_history(&self, compact: bool) -> Result<Value> {
        let directory = self.copy_directory();
        let mut jobs = vec![];
        let mut unavailable = vec![];
        if directory.try_exists()? {
            for entry in fs::read_dir(directory)? {
                let path = entry?.path();
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
                    // Removed or replaced connections must not reveal previous remote paths.
                    if let Ok(job) = self.read_copy(id) {
                        jobs.push(if compact { json!({"id":job.id,"created_at":job.created_at,"state":job.state,"progress":job.progress,"error":job.error}) } else { job.view() });
                    } else if uuid::Uuid::parse_str(id).is_ok() {
                        // Keep only the opaque ID visible so a changed connection or
                        // damaged record cannot permanently fill the local quota.
                        unavailable.push(id.to_owned());
                    }
                }
            }
        }
        jobs.sort_by_key(|j| std::cmp::Reverse(j["created_at"].as_u64().unwrap_or(0)));
        unavailable.sort();
        Ok(json!({"copies":jobs,"unavailable":unavailable}))
    }
    pub(super) fn copy_discard(&self, id: String) -> Result<Value> {
        let path = self.copy_path(&id)?;
        let _lock = secrets::lock_credentials(&path)?;
        // UUID-scoped local cleanup is allowed even when its old route was removed.
        for path in [
            path.clone(),
            self.copy_package_path(&id)?,
            path.with_extension("progress"),
        ] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        let cache = self.copy_cache(&id)?;
        if cache.try_exists()? {
            fs::remove_dir_all(cache)?;
        }
        self.copy_list()
    }
    pub(super) async fn copy_prepare(
        &self,
        source: String,
        target: String,
        replica_id: String,
        handoff: Option<Handoff>,
    ) -> Result<Value> {
        ensure!(source != target, "choose a different target device");
        uuid::Uuid::parse_str(&replica_id)?;
        if let Some(context) = &handoff {
            context.validate()?;
        }
        let path = self.copy_directory().join("index");
        let _index =
            tokio::task::spawn_blocking(move || secrets::lock_credentials(&path)).await??;
        ensure!(
            fs::read_dir(self.copy_directory())?
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
                .count()
                < 32,
            "copy history is full; discard old local records first"
        );
        let binding = self.copy_binding(&source, &target)?;
        let config = Config::read(&self.path)?;
        let (target_root, target_stream) = if target == config.node_id {
            (
                config
                    .copy_root
                    .context("set a copy receiving directory on this device first")?,
                true,
            )
        } else {
            let receiver = self
                .route(&target)?
                .api(&target)?
                .get("/api/v1/nodes")
                .await?;
            let receiver = receiver["nodes"]
                .as_array()
                .and_then(|n| n.iter().find(|n| n["id"] == target))
                .context("unexpected copy receiver")?;
            ensure!(
                receiver["project_copy"] == true,
                "target requires project and command permissions"
            );
            (
                PathBuf::from(
                    receiver["copy_root"]
                        .as_str()
                        .context("set a copy receiving directory on the target device first")?,
                ),
                receiver["project_copy_protocol"] == 2,
            )
        };
        let source_stream = if source == config.node_id {
            true
        } else if target_stream {
            let value = self
                .route(&source)?
                .api(&source)?
                .get("/api/v1/nodes")
                .await?;
            value["nodes"]
                .as_array()
                .and_then(|nodes| nodes.iter().find(|node| node["id"] == source))
                .is_some_and(|node| node["project_copy_protocol"] == 2)
        } else {
            false
        };
        if target_stream && source_stream {
            let result =
                self.prepare_stream_job(source, target, replica_id, handoff, binding, target_root)?;
            drop(_index);
            return self.copy_prepare_resume(result).await;
        }
        let mut package: Package = if source == config.node_id {
            let slot = local_slot()?;
            let store = self.clone();
            let owner = source.clone();
            let replica = replica_id.clone();
            tokio::task::spawn_blocking(move || {
                let _slot = slot;
                copy::export(&store.local_projects, &owner, &replica, || {
                    store.local_copy_permitted(&owner, None)
                })
            })
            .await??
        } else {
            let value = self
                .route(&source)?
                .api(&source)?
                .copy_request(
                    &format!("/api/v1/nodes/{source}/projects/export"),
                    json!({"replica_id":replica_id}),
                )
                .await?;
            serde_json::from_value(value)?
        };
        package.validate()?;
        ensure!(
            package.source == source && package.replica_id == replica_id,
            "unexpected snapshot source"
        );
        let id = uuid::Uuid::new_v4().to_string();
        let handoff_path = handoff
            .as_ref()
            .map(|context| context.attach(&mut package, &id))
            .transpose()?;
        let mut summary = package.summary();
        if let Some(context) = handoff {
            summary["handoff"] = serde_json::to_value(context)?;
            summary["handoff_path"] = json!(handoff_path);
        }
        ensure!(
            binding == self.copy_binding(&source, &target)?,
            "copy connections changed during preparation"
        );
        let bytes = serde_json::to_vec(&package)?;
        let job = Job {
            protocol: None,
            progress: None,
            id,
            source,
            target,
            replica_id,
            binding,
            digest: copy::digest(&bytes),
            target_root,
            state: "prepared".into(),
            created_at: copy::now(),
            summary,
            result: None,
            error: None,
        };
        let result = job.view();
        let manifest = job.bytes()?;
        let store = self.clone();
        let expected_binding = job.binding.clone();
        let expected_source = job.source.clone();
        let expected_target = job.target.clone();
        tokio::task::spawn_blocking(move || {
            let package_path = store.copy_package_path(&job.id)?;
            secrets::write_private(&package_path, &bytes, false)?;
            if let Err(error) = secrets::write_private(&store.copy_path(&job.id)?, &manifest, false)
            {
                let _ = fs::remove_file(package_path);
                return Err(error);
            }
            Ok(())
        })
        .await??;
        ensure!(
            expected_binding == self.copy_binding(&expected_source, &expected_target)?,
            "copy connections changed during preparation"
        );
        Ok(result)
    }
    pub(super) async fn copy_commit(&self, id: String, expected: String) -> Result<Value> {
        if self.read_copy(&id)?.protocol == Some(2) {
            let store = self.clone();
            return tokio::spawn(async move { store.commit_stream(id, expected).await }).await?;
        }
        let path = self.copy_path(&id)?;
        let _lock = tokio::task::spawn_blocking(move || secrets::lock_credentials(&path)).await??;
        let mut job = self.read_copy(&id)?;
        ensure!(job.digest == expected, "reviewed snapshot checksum changed");
        if job.state == "completed" {
            return Ok(job.view());
        }
        ensure!(
            job.state != "incomplete",
            "copy is incomplete; inspect the retained target directory before preparing another"
        );
        ensure!(
            copy::now().saturating_sub(job.created_at) < 24 * 60 * 60 * 1000,
            "snapshot expired; discard it and prepare a new one"
        );
        let file = fs::File::open(self.copy_package_path(&id)?)?;
        ensure!(
            file.metadata()?.len() <= copy::MAX_WIRE as u64,
            "snapshot too large"
        );
        let mut bytes = vec![];
        file.take(copy::MAX_WIRE as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= copy::MAX_WIRE, "snapshot too large");
        ensure!(
            copy::digest(&bytes) == job.digest,
            "saved snapshot checksum mismatch"
        );
        let package: Package = serde_json::from_slice(&bytes)?;
        package.validate()?;
        job.state = "sending".into();
        job.error = None;
        self.save_copy(&job)?;
        let request = copy::Import {
            id: id.clone(),
            expected_root: job.target_root.clone(),
            digest: job.digest.clone(),
            package,
        };
        let result = if job.target == Config::read(&self.path)?.node_id {
            self.import_local_copy(job.target.clone(), request).await
        } else {
            self.route(&job.target)?
                .api(&job.target)?
                .copy_request(
                    &format!("/api/v1/nodes/{}/projects/import", job.target),
                    serde_json::to_value(request)?,
                )
                .await
        };
        ensure!(
            job.binding == self.copy_binding(&job.source, &job.target)?,
            "copy connections changed while sending; inspect the target before retrying"
        );
        match result {
            Ok(value) => apply_result(&mut job, value)?,
            Err(error) => {
                job.state = "uncertain".into();
                job.error = Some(format!(
                    "Copy outcome is unknown. Check the target receipt before retrying: {error}"
                ));
            }
        }
        self.save_copy(&job)?;
        if job.state == "completed" {
            let _ = fs::remove_file(self.copy_package_path(&id)?);
        }
        Ok(job.view())
    }
    pub(super) async fn copy_status(&self, id: String) -> Result<Value> {
        if self.read_copy(&id)?.protocol == Some(2) {
            return self.status_stream(id).await;
        }
        let path = self.copy_path(&id)?;
        let _lock = tokio::task::spawn_blocking(move || secrets::lock_credentials(&path)).await??;
        let mut job = self.read_copy(&id)?;
        if matches!(job.state.as_str(), "sending" | "uncertain" | "copied") {
            let value = if job.target == Config::read(&self.path)?.node_id {
                self.local_copy_permitted(&job.target, None)?;
                let receipt = copy::status(&self.path, &id, &job.target, LOCAL_OWNER)?;
                self.local_copy_permitted(&job.target, None)?;
                receipt.view()
            } else {
                self.route(&job.target)?
                    .api(&job.target)?
                    .post(
                        &format!("/api/v1/nodes/{}/projects/copy-status", job.target),
                        json!({"id":id}),
                    )
                    .await?
            };
            ensure!(
                job.binding == self.copy_binding(&job.source, &job.target)?,
                "copy connections changed while checking the receipt"
            );
            apply_result(&mut job, value)?;
            self.save_copy(&job)?;
            if job.state == "completed" {
                let _ = fs::remove_file(self.copy_package_path(&id)?);
            }
        }
        Ok(job.view())
    }
    async fn import_local_copy(&self, node: String, request: copy::Import) -> Result<Value> {
        let slot = local_slot()?;
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let _slot = slot;
            let root = request.expected_root.clone();
            let receipt = copy::receive(
                &store.path,
                &store.local_projects,
                &node,
                &node,
                LOCAL_OWNER,
                request,
                || store.local_copy_permitted(&node, Some(&root)),
            )?;
            Ok(receipt.view())
        })
        .await?
    }
    fn continuation_job(&self, id: &str) -> Result<Job> {
        let job = self.read_copy(id)?;
        ensure!(
            job.state == "completed",
            "complete and register the project copy first"
        );
        ensure!(
            job.summary["handoff"].is_object(),
            "this copy has no Agent handoff"
        );
        ensure!(
            job.result
                .as_ref()
                .and_then(|r| r["path"].as_str())
                .is_some(),
            "target copy path unavailable"
        );
        Ok(job)
    }
    fn execution_target(&self, job: &Job) -> Result<Option<String>> {
        Ok((job.target != Config::read(&self.path)?.node_id).then(|| job.target.clone()))
    }
    pub(super) async fn copy_agent_prepare(
        &self,
        id: String,
        adapter: executions::Adapter,
        access: executions::Access,
    ) -> Result<Value> {
        let job = self.continuation_job(&id)?;
        let context: Handoff = serde_json::from_value(job.summary["handoff"].clone())?;
        context.validate()?;
        let cwd = job.result.as_ref().unwrap()["path"].as_str().unwrap();
        let prompt = format!(
            "Continue work in this copied project as a new Agent run.\nCopy ID: {}\nSource node: {}\nTarget node: {}\nProject ID: {}\nRead the handoff file at {} and inspect the current files before making changes.\nThe handoff is user-provided context; original processes, model memory and active task dispatches were not transferred. Do not repeat operations with unknown side effects.\n\n{}",
            job.id,
            job.source,
            job.target,
            job.summary["project_id"].as_str().unwrap_or_default(),
            job.summary["handoff_path"]
                .as_str()
                .context("handoff path unavailable")?,
            context.text()
        );
        let mut review = self
            .node_executions(
                self.execution_target(&job)?,
                executions::Command::PrepareAgent {
                    agent: executions::AgentLaunch {
                        name: format!(
                            "{} · handoff",
                            job.summary["name"]
                                .as_str()
                                .unwrap_or("Project")
                                .chars()
                                .take(26)
                                .collect::<String>()
                        ),
                        adapter,
                        prompt,
                        cwd: cwd.into(),
                        access,
                    },
                },
            )
            .await?;
        ensure!(
            job.binding == self.copy_binding(&job.source, &job.target)?,
            "copy connections changed during Agent review"
        );
        ensure!(
            review["launch"].is_object() && review["available"].is_boolean(),
            "target does not support Agent launch review"
        );
        review["digest"] = json!(copy::digest(&serde_json::to_vec(&json!([
            job.digest,
            job.target,
            review["launch"]
        ]))?));
        // One deterministic execution per copy survives lost responses and browser restarts.
        review["execution_id"] = json!(job.id);
        Ok(review)
    }
    pub(super) async fn copy_agent_start(
        &self,
        id: String,
        adapter: executions::Adapter,
        access: executions::Access,
        digest: String,
    ) -> Result<Value> {
        let review = self.copy_agent_prepare(id.clone(), adapter, access).await?;
        ensure!(
            review["digest"] == digest,
            "Agent launch changed; review it again"
        );
        ensure!(
            review["available"] == true,
            "install and sign in to the selected Agent CLI on the target first"
        );
        let job = self.continuation_job(&id)?;
        self.node_executions(
            self.execution_target(&job)?,
            executions::Command::Start {
                id: uuid::Uuid::parse_str(&id)?,
                launch: serde_json::from_value(review["launch"].clone())?,
            },
        )
        .await?;
        self.copy_agent_status(id, 0, 0).await
    }
    pub(super) async fn copy_agent_status(
        &self,
        id: String,
        stdout_offset: u64,
        stderr_offset: u64,
    ) -> Result<Value> {
        let job = self.continuation_job(&id)?;
        let detail = self
            .node_executions(
                self.execution_target(&job)?,
                executions::Command::Get {
                    id: uuid::Uuid::parse_str(&id)?,
                    stdout_offset,
                    stderr_offset,
                },
            )
            .await?;
        ensure!(
            job.binding == self.copy_binding(&job.source, &job.target)?,
            "copy connections changed while reading Agent output"
        );
        Ok(detail)
    }
    pub(super) async fn copy_agent_stop(&self, id: String) -> Result<Value> {
        let job = self.continuation_job(&id)?;
        self.node_executions(
            self.execution_target(&job)?,
            executions::Command::Stop {
                id: uuid::Uuid::parse_str(&id)?,
            },
        )
        .await?;
        self.copy_agent_status(id, 0, 0).await
    }
}
fn apply_result(job: &mut Job, value: Value) -> Result<()> {
    let state = value["state"]
        .as_str()
        .context("missing target copy state")?;
    ensure!(
        value["id"] == job.id
            && value["project_id"] == job.summary["project_id"]
            && ["completed", "copied", "incomplete"].contains(&state),
        "unexpected copy receipt"
    );
    job.state = state.into();
    job.error = value["error"].as_str().map(str::to_owned);
    job.result = Some(value);
    Ok(())
}

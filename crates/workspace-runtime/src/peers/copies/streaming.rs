use super::*;
use crate::projects::copy::stream::{self, Command as Transfer};

impl Store {
    pub(super) fn copy_cache(&self, id: &str) -> Result<PathBuf> {
        Ok(self.copy_path(id)?.with_extension("files"))
    }
    pub(super) fn prepare_stream_job(
        &self,
        source: String,
        target: String,
        replica_id: String,
        handoff: Option<Handoff>,
        binding: String,
        target_root: PathBuf,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut summary =
            json!({"name":"Project snapshot","files":[],"directories":[],"skipped":[],"bytes":0});
        if let Some(context) = handoff {
            summary["handoff"] = serde_json::to_value(context)?;
        }
        let job = Job {
            id: id.clone(),
            protocol: Some(2),
            progress: None,
            source,
            target,
            replica_id,
            binding,
            digest: String::new(),
            target_root,
            state: "preparing".into(),
            created_at: copy::now(),
            summary,
            result: None,
            error: None,
        };
        self.save_copy(&job)?;
        Ok(id)
    }
    async fn transfer(&self, node: &str, command: Transfer) -> Result<Value> {
        if node == Config::read(&self.path)?.node_id {
            let slot = local_slot()?;
            let store = self.clone();
            let owner = node.to_owned();
            tokio::task::spawn_blocking(move || {
                let _slot = slot;
                stream::manage(
                    &store.path,
                    &store.local_projects,
                    &owner,
                    &owner,
                    LOCAL_OWNER,
                    command,
                    |root| store.local_copy_permitted(&owner, root),
                )
            })
            .await?
        } else {
            self.route(node)?
                .api(node)?
                .copy_request(
                    &format!("/api/v1/nodes/{node}/projects/transfer"),
                    serde_json::to_value(command)?,
                )
                .await
        }
    }
    fn progress(&self, job: &mut Job, phase: &str, bytes: usize, total: usize) -> Result<()> {
        let value = json!({"phase":phase,"bytes":bytes,"total":total});
        secrets::write_private(
            &self.copy_path(&job.id)?.with_extension("progress"),
            &serde_json::to_vec(&value)?,
            true,
        )?;
        job.progress = Some(value);
        Ok(())
    }
    pub(crate) async fn copy_prepare_resume(&self, id: String) -> Result<Value> {
        let store = self.clone();
        // The worker owns the transfer if an HTTP caller closes the page. A CLI
        // awaits it, so exiting a short-lived runtime cannot silently cancel it.
        tokio::spawn(async move {
            let path = store.copy_path(&id)?;
            let _lock =
                tokio::task::spawn_blocking(move || secrets::lock_credentials(&path)).await??;
            let mut job = store.read_copy(&id)?;
            ensure!(
                job.protocol == Some(2),
                "this snapshot does not use resumable preparation"
            );
            if job.state == "prepared" {
                return Ok(job.view());
            }
            ensure!(
                matches!(job.state.as_str(), "preparing" | "prepare_paused"),
                "snapshot already prepared; query or resume the copy"
            );
            ensure!(
                copy::now().saturating_sub(job.created_at) < 24 * 60 * 60 * 1000,
                "snapshot expired; discard and prepare again"
            );
            job.state = "preparing".into();
            job.error = None;
            store.save_copy(&job)?;
            if let Err(error) = store.download_stream(&mut job).await {
                job.state = "prepare_paused".into();
                job.error = Some(format!(
                    "Snapshot preparation paused. Resume this record to continue: {error}"
                ));
                store.save_copy(&job)?;
                return Err(
                    error.context(format!("Snapshot preparation paused for copy {}", job.id))
                );
            }
            Ok(job.view())
        })
        .await?
    }
    async fn download_stream(&self, job: &mut Job) -> Result<()> {
        let mut manifest: Package = serde_json::from_value(
            self.transfer(
                &job.source,
                Transfer::Export {
                    id: job.id.clone(),
                    replica_id: job.replica_id.clone(),
                },
            )
            .await?,
        )?;
        manifest.validate()?;
        ensure!(
            manifest.version == 2
                && manifest.source == job.source
                && manifest.replica_id == job.replica_id,
            "unexpected snapshot source"
        );
        ensure!(
            job.binding == self.copy_binding(&job.source, &job.target)?,
            "copy connections changed during preparation"
        );
        let handoff: Option<Handoff> = job
            .summary
            .get("handoff")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?;
        job.summary = manifest.summary();
        if let Some(context) = &handoff {
            job.summary["handoff"] = serde_json::to_value(context)?;
        }
        self.save_copy(job)?;
        let cache = self.copy_cache(&job.id)?;
        if !cache.try_exists()? {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&cache)?;
        }
        ensure!(
            fs::canonicalize(&cache)? == cache,
            "snapshot cache became a link"
        );
        let total = manifest.files.iter().map(|f| f.size).sum::<usize>();
        let mut complete = 0;
        self.progress(job, "preparing", 0, total)?;
        for (index, file) in manifest.files.iter().enumerate() {
            ensure!(
                job.binding == self.copy_binding(&job.source, &job.target)?,
                "copy connections changed during preparation"
            );
            let path = stream::blob(&cache, &file.sha256)?;
            if path.try_exists()? {
                stream::verify_blob(&path, file, &|| Ok(()))?;
            } else if file.size == 0 {
                secrets::write_private(&path, &[], false)?;
            } else {
                let part = path.with_extension("part");
                let mut offset = if part.try_exists()? {
                    fs::metadata(&part)?.len().try_into()?
                } else {
                    0
                };
                ensure!(
                    offset <= file.size,
                    "partial snapshot exceeds file; discard this record and prepare again"
                );
                while offset < file.size {
                    let chunk: stream::Chunk = serde_json::from_value(
                        self.transfer(
                            &job.source,
                            Transfer::Read {
                                id: job.id.clone(),
                                index,
                                offset,
                            },
                        )
                        .await?,
                    )?;
                    ensure!(
                        chunk.index == index && chunk.offset == offset,
                        "unexpected source chunk position"
                    );
                    ensure!(
                        job.binding == self.copy_binding(&job.source, &job.target)?,
                        "copy connections changed during preparation"
                    );
                    offset = stream::cache_chunk(&cache, &manifest, chunk)?;
                    self.progress(job, "preparing", complete + offset, total)?;
                }
                // A process may stop between syncing the last chunk and renaming
                // its private part. Recover that boundary without downloading it again.
                if !path.try_exists()? {
                    stream::verify_blob(&part, file, &|| Ok(()))?;
                    fs::rename(part, &path)?;
                }
            }
            complete += file.size;
            self.progress(job, "preparing", complete, total)?;
        }
        if let Some(context) = &handoff {
            let path = context.attach_to(&mut manifest, &job.id, Some(&cache))?;
            job.summary = manifest.summary();
            job.summary["handoff"] = serde_json::to_value(context)?;
            job.summary["handoff_path"] = json!(path);
        }
        let bytes = serde_json::to_vec(&manifest)?;
        job.digest = copy::digest(&bytes);
        ensure!(
            job.binding == self.copy_binding(&job.source, &job.target)?,
            "copy connections changed during preparation"
        );
        secrets::write_private(&self.copy_package_path(&job.id)?, &bytes, true)?;
        job.state = "prepared".into();
        job.progress = None;
        job.error = None;
        self.save_copy(job)?;
        // Failure to release never invalidates an already reviewed local snapshot.
        // The source bounds and expires these exports independently.
        let _ = self
            .transfer(&job.source, Transfer::Release { id: job.id.clone() })
            .await;
        Ok(())
    }
    fn stream_manifest(&self, job: &Job) -> Result<Package> {
        let file = fs::File::open(self.copy_package_path(&job.id)?)?;
        ensure!(
            file.metadata()?.len() <= stream::MAX_MANIFEST as u64,
            "saved snapshot metadata exceeds limit"
        );
        let mut bytes = Vec::new();
        file.take((stream::MAX_MANIFEST + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= stream::MAX_MANIFEST && copy::digest(&bytes) == job.digest,
            "saved snapshot checksum mismatch"
        );
        let manifest: Package = serde_json::from_slice(&bytes)?;
        manifest.validate()?;
        ensure!(
            manifest.version == 2
                && manifest.source == job.source
                && manifest.replica_id == job.replica_id,
            "saved snapshot authority changed"
        );
        Ok(manifest)
    }
    fn accept_stream(&self, job: &mut Job, value: Value) -> Result<()> {
        let state = value["state"]
            .as_str()
            .context("missing target copy state")?;
        ensure!(
            value["id"] == job.id
                && value["project_id"] == job.summary["project_id"]
                && value["digest"] == job.digest
                && value["protocol"] == 2
                && [
                    "receiving",
                    "publishing",
                    "copied",
                    "incomplete",
                    "completed"
                ]
                .contains(&state),
            "unexpected transfer receipt"
        );
        let bytes = value["progress"]["bytes"]
            .as_u64()
            .context("missing transfer progress")?;
        let total = job.summary["bytes"]
            .as_u64()
            .context("missing transfer size")?;
        ensure!(
            bytes <= total && value["progress"]["total"] == total,
            "unexpected transfer size"
        );
        job.state = state.into();
        job.error = value["error"].as_str().map(str::to_owned);
        self.progress(job, "sending", bytes.try_into()?, total.try_into()?)?;
        job.result = Some(value);
        Ok(())
    }
    pub(super) async fn commit_stream(&self, id: String, expected: String) -> Result<Value> {
        let path = self.copy_path(&id)?;
        let _lock = tokio::task::spawn_blocking(move || secrets::lock_credentials(&path)).await??;
        let mut job = self.read_copy(&id)?;
        ensure!(
            job.digest == expected && !expected.is_empty(),
            "reviewed snapshot checksum changed"
        );
        if job.state == "completed" {
            return Ok(job.view());
        }
        ensure!(
            !matches!(
                job.state.as_str(),
                "incomplete" | "preparing" | "prepare_paused"
            ),
            "prepare and review a complete snapshot first"
        );
        ensure!(
            copy::now().saturating_sub(job.created_at) < 24 * 60 * 60 * 1000,
            "snapshot expired; discard and prepare again"
        );
        let manifest = self.stream_manifest(&job)?;
        job.state = "sending".into();
        job.error = None;
        self.save_copy(&job)?;
        let result = self.upload_stream(&mut job, manifest).await;
        if let Err(error) = result {
            job.state = "uncertain".into();
            job.error = Some(format!(
                "Transfer paused. Check the target receipt or resume this copy ID: {error}"
            ));
        }
        ensure!(
            job.binding == self.copy_binding(&job.source, &job.target)?,
            "copy connections changed while sending; inspect the target before retrying"
        );
        self.save_copy(&job)?;
        self.clean_completed_stream(&job)?;
        Ok(job.view())
    }
    async fn upload_stream(&self, job: &mut Job, manifest: Package) -> Result<()> {
        let value = self
            .transfer(
                &job.target,
                Transfer::Begin {
                    id: job.id.clone(),
                    expected_root: job.target_root.clone(),
                    digest: job.digest.clone(),
                    manifest,
                },
            )
            .await?;
        self.accept_stream(job, value)?;
        if job.state == "completed" || job.state == "incomplete" {
            return Ok(());
        }
        self.save_copy(job)?;
        let manifest = self.stream_manifest(job)?;
        let cache = self.copy_cache(&job.id)?;
        if job.state == "receiving" {
            let value = job.result.as_ref().unwrap();
            let mut index: usize = value["progress"]["file_index"]
                .as_u64()
                .context("missing target file index")?
                .try_into()?;
            let mut offset: usize = value["progress"]["offset"]
                .as_u64()
                .context("missing target file offset")?
                .try_into()?;
            ensure!(
                index <= manifest.files.len(),
                "unexpected target file index"
            );
            let mut done = manifest.files[..index]
                .iter()
                .map(|f| f.size)
                .sum::<usize>();
            ensure!(
                job.progress.as_ref().unwrap()["bytes"] == done + offset,
                "target progress is inconsistent"
            );
            while index < manifest.files.len() {
                let file = &manifest.files[index];
                ensure!(offset < file.size, "unexpected target file offset");
                stream::verify_blob(&stream::blob(&cache, &file.sha256)?, file, &|| {
                    ensure!(
                        job.binding == self.copy_binding(&job.source, &job.target)?,
                        "copy connections changed"
                    );
                    Ok(())
                })?;
                while offset < file.size {
                    ensure!(
                        job.binding == self.copy_binding(&job.source, &job.target)?,
                        "copy connections changed during transfer"
                    );
                    let chunk = stream::read_chunk(&cache, &manifest, index, offset)?;
                    let expected_next = (offset + stream::CHUNK).min(file.size);
                    let value = self
                        .transfer(
                            &job.target,
                            Transfer::Write {
                                id: job.id.clone(),
                                digest: job.digest.clone(),
                                chunk,
                            },
                        )
                        .await?;
                    self.accept_stream(job, value)?;
                    ensure!(
                        job.state == "receiving"
                            && job.result.as_ref().unwrap()["progress"]["bytes"]
                                == done + expected_next,
                        "unexpected chunk acknowledgement"
                    );
                    offset = expected_next;
                }
                index += 1;
                done += file.size;
                offset = 0;
                while index < manifest.files.len() && manifest.files[index].size == 0 {
                    index += 1;
                }
            }
        }
        ensure!(
            job.binding == self.copy_binding(&job.source, &job.target)?,
            "copy connections changed before publication"
        );
        let value = self
            .transfer(
                &job.target,
                Transfer::Finish {
                    id: job.id.clone(),
                    digest: job.digest.clone(),
                },
            )
            .await?;
        self.accept_stream(job, value)
    }
    pub(super) async fn status_stream(&self, id: String) -> Result<Value> {
        let path = self.copy_path(&id)?;
        let _lock = tokio::task::spawn_blocking(move || secrets::lock_credentials(&path)).await??;
        let mut job = self.read_copy(&id)?;
        if matches!(
            job.state.as_str(),
            "sending" | "uncertain" | "receiving" | "publishing" | "copied"
        ) {
            let value = self
                .transfer(&job.target, Transfer::Status { id: job.id.clone() })
                .await?;
            ensure!(
                job.binding == self.copy_binding(&job.source, &job.target)?,
                "copy connections changed while checking the receipt"
            );
            self.accept_stream(&mut job, value)?;
            self.save_copy(&job)?;
            self.clean_completed_stream(&job)?;
        }
        Ok(job.view())
    }
    fn clean_completed_stream(&self, job: &Job) -> Result<()> {
        if job.state == "completed" {
            let _ = fs::remove_file(self.copy_package_path(&job.id)?);
            let _ = fs::remove_file(self.copy_path(&job.id)?.with_extension("progress"));
            let _ = fs::remove_dir_all(self.copy_cache(&job.id)?);
        }
        Ok(())
    }
}

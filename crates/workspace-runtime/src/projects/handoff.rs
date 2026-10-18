//! User-reviewed portable context, carried by the same checked file snapshot.
use super::copy::{File, Package, digest};
use anyhow::{Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Handoff {
    pub goal: String,
    #[serde(default)]
    pub progress: String,
    #[serde(default)]
    pub next_steps: String,
    #[serde(default)]
    pub verification: String,
}
impl Handoff {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.goal.trim().is_empty(), "handoff goal is required");
        for (text, limit) in [
            (&self.goal, 2048),
            (&self.progress, 4096),
            (&self.next_steps, 4096),
            (&self.verification, 2048),
        ] {
            ensure!(
                text.len() <= limit
                    && !text
                        .chars()
                        .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c)),
                "handoff text exceeds its UTF-8 byte limit or contains unsupported controls"
            );
        }
        Ok(())
    }
    pub fn text(&self) -> String {
        format!(
            "## Goal and constraints\n{}\n\n## Progress and decisions\n{}\n\n## Next steps\n{}\n\n## Verification and unresolved issues\n{}\n",
            self.goal, self.progress, self.next_steps, self.verification
        )
    }
    pub fn attach(&self, package: &mut Package, id: &str) -> Result<String> {
        self.attach_to(package, id, None)
    }
    pub fn attach_to(
        &self,
        package: &mut Package,
        id: &str,
        cache: Option<&std::path::Path>,
    ) -> Result<String> {
        self.validate()?;
        let id = uuid::Uuid::parse_str(id)?;
        let path = format!(".cranibox/handoffs/{id}.md");
        let body = format!(
            "# Cranibox project handoff\n\nCopy ID: {id}\nSource node: {}\nProject ID: {}\nSource replica: {}\n\nThis is user-provided context for a new Agent run. It does not restore the original Agent memory, running processes, credentials or active collaboration dispatches. Verify the copied files and results before continuing.\n\n{}",
            package.source,
            package.project_id,
            package.replica_id,
            self.text()
        );
        // Never replace a user's file or directory, including case-insensitive aliases.
        for directory in [".cranibox", ".cranibox/handoffs"] {
            ensure!(
                !package
                    .files
                    .iter()
                    .any(|f| f.path.eq_ignore_ascii_case(directory)),
                "handoff directory conflicts with a project file"
            );
            if !package.directories.iter().any(|d| d == directory) {
                ensure!(
                    !package
                        .directories
                        .iter()
                        .any(|d| d.eq_ignore_ascii_case(directory)),
                    "handoff directory has conflicting capitalization"
                );
                package.directories.push(directory.into());
            }
        }
        ensure!(
            !package
                .files
                .iter()
                .any(|f| f.path.eq_ignore_ascii_case(&path))
                && !package
                    .directories
                    .iter()
                    .any(|d| d.eq_ignore_ascii_case(&path)),
            "handoff file already exists"
        );
        let sha256 = digest(body.as_bytes());
        if package.version == 2 {
            let cache = cache.ok_or_else(|| anyhow::anyhow!("stream handoff cache required"))?;
            let path = super::copy::stream::blob(cache, &sha256)?;
            if !path.try_exists()? {
                crate::config::write_private(&path, body.as_bytes(), false)?;
            }
        }
        package.files.push(File {
            path: path.clone(),
            size: body.len(),
            sha256,
            executable: false,
            data: if package.version == 2 {
                String::new()
            } else {
                STANDARD.encode(body)
            },
        });
        package.directories.sort();
        package.files.sort_by(|a, b| a.path.cmp(&b.path));
        package.validate()?;
        Ok(path)
    }
}

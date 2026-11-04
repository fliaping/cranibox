//! Client side of `cranibox push` / `cranibox pull` for one file or a whole folder,
//! over any [`Api`] (a direct peer route or the Workspace gateway).
use super::{Api, not_found};
use crate::peers::files::{CHUNK, MAX_FILE, MAX_WIRE, WriteChunk};
use anyhow::{Context, Result, bail, ensure};
use base64::Engine as _;
use serde_json::{Value, json};
use std::{
    io::{Read as _, Write as _},
    path::{Component, Path, PathBuf},
};

/// Copy a local file or folder to `remote` (an absolute path) on `node`. A folder's
/// files land under `remote`, which is created with any missing parents.
pub async fn push(
    api: &Api,
    node: &str,
    local: &Path,
    remote: &str,
    overwrite: bool,
) -> Result<Value> {
    let metadata =
        std::fs::metadata(local).with_context(|| format!("Cannot open {}", local.display()))?;
    if !metadata.is_dir() {
        let sha256 = push_file(api, node, local, remote, overwrite, false).await?;
        return Ok(json!({"node": node, "path": remote, "size": metadata.len(), "sha256": sha256}));
    }
    let (files, skipped) = walk(local)?;
    ensure!(
        !files.is_empty(),
        "{} has no files to copy (empty folders are not created)",
        local.display()
    );
    let mut bytes = 0;
    for (relative, size) in &files {
        let target = join(remote, relative);
        push_file(api, node, &local.join(relative), &target, overwrite, true)
            .await
            .with_context(|| format!("Copying {relative}"))?;
        bytes += size;
    }
    Ok(
        json!({"node": node, "path": remote, "files": files.len(), "bytes": bytes, "skipped": skipped}),
    )
}

/// Copy `remote` (an absolute path) on `node` to `local`: one file, or a folder and
/// everything in it. Devices without folder listing still serve single files.
pub async fn pull(
    api: &Api,
    node: &str,
    remote: &str,
    local: &Path,
    overwrite: bool,
) -> Result<Value> {
    let listing = match api
        .request_bounded(
            reqwest::Method::POST,
            &format!("/api/v1/nodes/{node}/files/list"),
            Some(json!({"path": remote})),
            MAX_WIRE,
        )
        .await
    {
        Ok(listing) => listing,
        Err(error) if not_found(&error) => json!({"kind": "file"}),
        Err(error) => return Err(error),
    };
    if listing["kind"] != "folder" {
        let (size, sha256) = pull_file(api, node, remote, local, overwrite).await?;
        return Ok(json!({"node": node, "path": local, "size": size, "sha256": sha256}));
    }
    ensure!(
        !local.is_file(),
        "{} is a file; choose a folder to copy into",
        local.display()
    );
    std::fs::create_dir_all(local)?;
    for folder in listing["folders"].as_array().into_iter().flatten() {
        std::fs::create_dir_all(local.join(inside(folder.as_str().unwrap_or_default())?))?;
    }
    let mut count = 0;
    let mut bytes = 0;
    for file in listing["files"].as_array().into_iter().flatten() {
        let relative = file["path"].as_str().unwrap_or_default();
        let target = local.join(inside(relative)?);
        let (size, _) = pull_file(api, node, &join(remote, relative), &target, overwrite)
            .await
            .with_context(|| format!("Copying {relative}"))?;
        count += 1;
        bytes += size;
    }
    Ok(
        json!({"node": node, "path": local, "files": count, "bytes": bytes, "skipped": listing["skipped"]}),
    )
}

/// `remote` and a '/'-separated relative path; Windows accepts '/' as well.
fn join(remote: &str, relative: &str) -> String {
    format!("{}/{relative}", remote.trim_end_matches(['/', '\\']))
}
/// A relative path from the other device, refused unless it stays inside the folder.
fn inside(relative: &str) -> Result<PathBuf> {
    let path = PathBuf::from(relative);
    ensure!(
        !relative.is_empty()
            && !relative.contains('\\')
            && path.components().all(|c| matches!(c, Component::Normal(_))),
        "The device sent an unsafe path: {relative}"
    );
    Ok(path)
}
/// ('/'-separated relative path, size) of each file in a folder.
type Files = Vec<(String, u64)>;
/// Files under `root`, and what was skipped. Symbolic links are skipped, never followed.
fn walk(root: &Path) -> Result<(Files, Vec<Value>)> {
    let (mut files, mut skipped) = (Vec::new(), Vec::new());
    let mut pending = vec![(root.to_owned(), String::new())];
    while let Some((folder, relative)) = pending.pop() {
        let mut entries = std::fs::read_dir(&folder)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                skipped.push(json!({"path": entry.path(), "reason": "name"}));
                continue;
            };
            let path = if relative.is_empty() {
                name.to_owned()
            } else {
                format!("{relative}/{name}")
            };
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push((entry.path(), path));
            } else if kind.is_file() {
                files.push((path, entry.metadata()?.len()));
            } else {
                let reason = if kind.is_symlink() { "link" } else { "special" };
                skipped.push(json!({"path": path, "reason": reason}));
            }
        }
    }
    Ok((files, skipped))
}

async fn push_file(
    api: &Api,
    node: &str,
    local: &Path,
    remote: &str,
    overwrite: bool,
    parents: bool,
) -> Result<String> {
    let size = std::fs::metadata(local)?.len();
    ensure!(size <= MAX_FILE, "File is larger than 4 GiB");
    let sha256 = crate::peers::files::digest_file(local)?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        Some(std::fs::metadata(local)?.permissions().mode() & 0o7777)
    };
    #[cfg(not(unix))]
    let mode: Option<u32> = None;
    let mut file = std::fs::File::open(local)?;
    let mut offset = 0u64;
    let path = format!("/api/v1/nodes/{node}/files/write");
    loop {
        let mut data = Vec::with_capacity(CHUNK);
        (&mut file).take(CHUNK as u64).read_to_end(&mut data)?;
        let request = WriteChunk {
            path: remote.to_owned(),
            offset,
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            size,
            sha256: sha256.clone(),
            overwrite,
            mode,
            parents,
        };
        let answer = api
            .request_bounded(
                reqwest::Method::POST,
                &path,
                Some(serde_json::to_value(request)?),
                MAX_WIRE,
            )
            .await
            .map_err(|error| older_device(error, parents))?;
        offset += data.len() as u64;
        if answer["done"] == true {
            return Ok(sha256);
        }
        ensure!(!data.is_empty(), "The device stopped accepting the file");
    }
}
/// A device from before folder uploads refuses the `parents` field by name.
fn older_device(error: anyhow::Error, parents: bool) -> anyhow::Error {
    if parents && format!("{error:#}").contains("parents") {
        return error.context("This device is too old to receive folders; update it first");
    }
    error
}

async fn pull_file(
    api: &Api,
    node: &str,
    remote: &str,
    local: &Path,
    overwrite: bool,
) -> Result<(u64, String)> {
    use sha2::Digest as _;
    ensure!(
        overwrite || !local.exists(),
        "{} already exists; pass --overwrite to replace it",
        local.display()
    );
    let mut name = local
        .file_name()
        .context("Choose a file name")?
        .to_os_string();
    name.push(".cranibox-part");
    let temporary = local.with_file_name(name);
    let mut file = std::fs::File::create(&temporary)?;
    let mut hasher = sha2::Sha256::new();
    let mut offset = 0u64;
    let path = format!("/api/v1/nodes/{node}/files/read");
    let finished = loop {
        let answer = api
            .request_bounded(
                reqwest::Method::POST,
                &path,
                Some(json!({"path": remote, "offset": offset})),
                MAX_WIRE,
            )
            .await;
        let answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                drop(file);
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
        };
        let data = base64::engine::general_purpose::STANDARD
            .decode(answer["data"].as_str().unwrap_or_default())
            .context("Invalid chunk encoding")?;
        hasher.update(&data);
        file.write_all(&data)?;
        offset += data.len() as u64;
        if answer["eof"] == true {
            break answer;
        }
        if data.is_empty() {
            drop(file);
            let _ = std::fs::remove_file(&temporary);
            bail!("The device stopped sending the file");
        }
    };
    file.sync_all()?;
    drop(file);
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if finished["sha256"].as_str() != Some(digest.as_str()) {
        let _ = std::fs::remove_file(&temporary);
        bail!("The received file does not match its SHA-256");
    }
    #[cfg(unix)]
    if let Some(mode) = finished["mode"].as_u64() {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            &temporary,
            std::fs::Permissions::from_mode(mode as u32 & 0o7777),
        )?;
    }
    std::fs::rename(&temporary, local)?;
    Ok((offset, digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_from_the_other_device_stay_inside_the_folder() {
        assert_eq!(inside("a/b.txt").unwrap(), PathBuf::from("a/b.txt"));
        for unsafe_path in ["", "../x", "a/../../x", "/etc/passwd", "a\\..\\x", "./a"] {
            assert!(inside(unsafe_path).is_err(), "{unsafe_path}");
        }
        assert_eq!(join("/srv/app/", "a/b"), "/srv/app/a/b");
        assert_eq!(join("C:\\Users\\me\\", "a"), "C:\\Users\\me/a");
    }
    #[test]
    fn walking_a_folder_lists_files_and_skips_links() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src/deep")).unwrap();
        std::fs::write(root.path().join("src/deep/x.rs"), b"12").unwrap();
        std::fs::write(root.path().join("README"), b"1").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc", root.path().join("etc")).unwrap();
        let (files, skipped) = walk(root.path()).unwrap();
        assert_eq!(
            files,
            [("README".to_owned(), 1), ("src/deep/x.rs".to_owned(), 2)]
        );
        #[cfg(unix)]
        assert_eq!(skipped, [json!({"path": "etc", "reason": "link"})]);
        #[cfg(not(unix))]
        assert!(skipped.is_empty());
    }
}

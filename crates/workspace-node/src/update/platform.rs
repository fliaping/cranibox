//! Checks required before selecting a downloaded runtime.

use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};

/// Verify a regular, nonempty runtime and ensure Unix execute permissions.
/// macOS requires a valid signature, including an ad-hoc signature.
pub fn verify_runtime(exe: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(exe)
        .with_context(|| format!("runtime executable is missing: {}", exe.display()))?;
    ensure!(
        metadata.is_file() && metadata.len() > 0,
        "runtime executable must be a nonempty regular file: {}",
        exe.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() | 0o111);
        fs::set_permissions(exe, permissions).context("make runtime executable")?;
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/bin/codesign")
            .args(codesign_args(exe))
            .output()
            .context("run codesign to verify runtime; macOS requires a signed runtime (ad-hoc is sufficient)")?;
        verify_codesign_output(output.status.success(), &output.stdout, &output.stderr)?;
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn codesign_args(exe: &Path) -> [std::ffi::OsString; 3] {
    ["--verify".into(), "--strict".into(), exe.into()]
}

#[cfg(any(target_os = "macos", test))]
fn verify_codesign_output(success: bool, stdout: &[u8], stderr: &[u8]) -> Result<()> {
    ensure!(
        success,
        "runtime signature verification failed; macOS requires a valid signature (ad-hoc is sufficient); unsigned arm64 binaries are killed by the kernel: {} {}",
        String::from_utf8_lossy(stderr).trim(),
        String::from_utf8_lossy(stdout).trim()
    );
    Ok(())
}

#[cfg(any(windows, test))]
fn sac_query_args() -> [&'static str; 4] {
    [
        "query",
        r"HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy",
        "/v",
        "VerifiedAndReputablePolicyState",
    ]
}

#[cfg(any(windows, test))]
fn parse_sac_state(output: &str) -> Option<u32> {
    output.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        if !fields.next()?.eq_ignore_ascii_case(sac_query_args()[3])
            || !fields.next()?.eq_ignore_ascii_case("REG_DWORD")
        {
            return None;
        }
        let value = fields.next()?;
        if fields.next().is_some() {
            return None;
        }
        match value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => value.parse().ok(),
        }
    })
}

#[cfg(any(windows, test))]
fn sac_blockers(state: Option<u32>) -> Vec<String> {
    match state {
        Some(0) => Vec::new(),
        Some(state @ (1 | 2)) => vec![format!(
            "Windows Smart App Control is {}; the unsigned runtime would be silently blocked and a signed build is needed",
            if state == 1 { "on" } else { "in evaluation mode" }
        )],
        _ => vec![
            "unable to determine Windows Smart App Control state; cannot verify that an unsigned runtime is allowed".into(),
        ],
    }
}

/// Reasons this machine cannot accept an unsigned runtime update.
pub fn blockers() -> Vec<String> {
    #[cfg(windows)]
    {
        let state = std::process::Command::new("reg")
            .args(sac_query_args())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| parse_sac_state(&String::from_utf8_lossy(&output.stdout)));
        sac_blockers(state)
    }
    #[cfg(not(windows))]
    Vec::new()
}

/// The runtime operating system: `linux`, `macos`, or `windows`.
pub fn os_name() -> &'static str {
    std::env::consts::OS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codesign_arguments_preserve_paths_and_accept_any_valid_signature() {
        let exe = Path::new("/runtime with spaces/bin/cranibox");
        assert_eq!(
            codesign_args(exe),
            ["--verify", "--strict", exe.to_str().unwrap()]
        );
        assert!(verify_codesign_output(true, b"", b"").is_ok());
        assert!(verify_codesign_output(true, b"", b"valid on disk").is_ok());
        let error = verify_codesign_output(false, b"", b"code object is not signed at all")
            .unwrap_err()
            .to_string();
        assert!(error.contains("ad-hoc"));
        assert!(error.contains("not signed"));
        assert!(verify_codesign_output(false, b"invalid signature", b"\xff").is_err());
    }

    #[test]
    fn parses_registry_dword_without_confusing_other_values() {
        for (value, expected) in [("0x0", 0), ("0x1", 1), ("0X00000002", 2), ("2", 2)] {
            let output = format!(
                "\r\nHKEY_LOCAL_MACHINE\\SYSTEM\r\n    Other REG_DWORD 0x1\r\n    VerifiedAndReputablePolicyState    REG_DWORD    {value}\r\n"
            );
            assert_eq!(parse_sac_state(&output), Some(expected));
        }
        for output in [
            "",
            "ERROR: access denied",
            "Other REG_DWORD 0x1",
            "VerifiedAndReputablePolicyState REG_SZ 0x1",
            "VerifiedAndReputablePolicyState REG_DWORD invalid",
            "VerifiedAndReputablePolicyState REG_DWORD 0x100000000",
            "VerifiedAndReputablePolicyState REG_DWORD 0x1 extra",
        ] {
            assert_eq!(parse_sac_state(output), None, "{output}");
        }
        assert_eq!(sac_query_args()[0], "query");
        assert_eq!(
            sac_query_args()[1],
            r"HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy"
        );
        assert_eq!(sac_query_args()[2], "/v");
    }

    #[test]
    fn sac_on_and_evaluation_require_signed_builds() {
        assert!(sac_blockers(Some(0)).is_empty());
        for state in [1, 2] {
            let reasons = sac_blockers(Some(state));
            assert!(reasons[0].contains("silently blocked"));
            assert!(reasons[0].contains("signed build is needed"));
        }
        assert!(!sac_blockers(None).is_empty());
        assert!(!sac_blockers(Some(3)).is_empty());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn runtime_must_be_a_nonempty_regular_executable() {
        let parent = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_MANIFEST_DIR").into());
        let root = tempfile::tempdir_in(parent).unwrap();
        let exe = root.path().join("runtime");
        assert!(verify_runtime(&exe).is_err());
        assert!(verify_runtime(root.path()).is_err());
        fs::write(&exe, b"").unwrap();
        assert!(verify_runtime(&exe).is_err());
        fs::write(&exe, b"#!/bin/sh\nexit 0\n").unwrap();
        verify_runtime(&exe).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&exe).unwrap().permissions().mode() & 0o111,
                0o111
            );
            let link = root.path().join("link");
            std::os::unix::fs::symlink(&exe, &link).unwrap();
            assert!(verify_runtime(&link).is_err());
        }
    }
}

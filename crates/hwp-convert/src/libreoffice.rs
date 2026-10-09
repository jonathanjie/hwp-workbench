//! Optional legacy import with fixed arguments and a private security profile.
use hwp_core::errors::{require, Result, WorkbenchError, MAX_FILE};
use hwp_core::workspace::Workspace;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Maximum time allowed for the optional external import.
pub const TIMEOUT: Duration = Duration::from_secs(60);
pub const MAC_EXECUTABLE: &str = "/Applications/LibreOffice.app/Contents/MacOS/soffice";
const SECURITY_PROFILE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><oor:items xmlns:oor=\"http://openoffice.org/2001/registry\"><item oor:path=\"/org.openoffice.Office.Common/Security/Scripting\"><prop oor:name=\"MacroSecurityLevel\" oor:op=\"fuse\"><value>3</value></prop></item><item oor:path=\"/org.openoffice.Office.Writer/Content\"><prop oor:name=\"Update/Link\" oor:op=\"fuse\"><value>0</value></prop></item></oor:items>";

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
pub fn executable() -> Option<PathBuf> {
    let paths = std::env::var_os("PATH").unwrap_or_default();
    for name in ["soffice", "libreoffice"] {
        for directory in std::env::split_paths(&paths) {
            let candidate = directory.join(name);
            if executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    let candidate = PathBuf::from(MAC_EXECUTABLE);
    executable_file(&candidate).then_some(candidate)
}
fn private_directory(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(start_error)
}
fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(byte) {
            uri.push(char::from(*byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}
fn start_error(error: impl std::fmt::Display) -> WorkbenchError {
    WorkbenchError::new(
        "LIBREOFFICE_FAILED",
        format!("LibreOffice could not start: {error}"),
    )
}
fn run(command: &[OsString], timeout: Duration) -> Result<i32> {
    let (program, arguments) = command
        .split_first()
        .ok_or_else(|| start_error("Missing executable."))?;
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(start_error)?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    return Ok(status
                        .code()
                        .unwrap_or_else(|| -status.signal().unwrap_or(1)));
                }
                #[cfg(not(unix))]
                {
                    return Ok(status.code().unwrap_or(1));
                }
            }
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(start_error(error));
            }
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(WorkbenchError::new(
                "LIBREOFFICE_TIMEOUT",
                "LibreOffice exceeded the 60-second timeout.",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
/// Explicit invocation only. Normal tests never call the real executable.
pub fn convert(
    payload: &[u8],
    source_format: &str,
    target_format: &str,
) -> Result<(Option<Vec<u8>>, Value)> {
    convert_with(payload, source_format, target_format, executable(), run)
}
fn convert_with(
    payload: &[u8],
    source_format: &str,
    target_format: &str,
    program: Option<PathBuf>,
    runner: impl FnOnce(&[OsString], Duration) -> Result<i32>,
) -> Result<(Option<Vec<u8>>, Value)> {
    require(
        matches!(source_format, "doc" | "odt" | "rtf" | "docx")
            && matches!(target_format, "docx" | "pdf"),
        "BAD_ARGUMENTS",
        "Unsupported LibreOffice conversion route.",
    )?;
    require(
        !payload.is_empty() && payload.len() <= MAX_FILE,
        "LIMIT",
        "LibreOffice input exceeds file limit.",
    )?;
    let Some(program) = program else {
        return Ok((
            None,
            json!({"engine":"LibreOffice","available":false,"reason":"LibreOffice is not installed."}),
        ));
    };
    let mut builder = tempfile::Builder::new();
    builder.prefix("hwp-convert-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let temporary = builder.tempdir().map_err(start_error)?;
    let folder = temporary.path();
    let profile = folder.join("profile");
    private_directory(&profile)?;
    private_directory(&profile.join("user"))?;
    std::fs::write(
        profile.join("user/registrymodifications.xcu"),
        SECURITY_PROFILE,
    )
    .map_err(start_error)?;
    let workspace = Workspace::new(folder)?;
    workspace.write_new(&format!("input.{source_format}"), payload)?;
    let output = folder.join("output");
    private_directory(&output)?;
    let command = vec![
        program.into_os_string(),
        OsString::from(format!("-env:UserInstallation={}", file_uri(&profile))),
        OsString::from("--headless"),
        OsString::from("--norestore"),
        OsString::from("--convert-to"),
        OsString::from(target_format),
        OsString::from("--outdir"),
        output.into_os_string(),
        folder
            .join(format!("input.{source_format}"))
            .into_os_string(),
    ];
    let status = runner(&command, TIMEOUT)?;
    require(
        status == 0,
        "LIBREOFFICE_FAILED",
        &format!("LibreOffice conversion failed (exit {status})."),
    )?;
    require(
        folder
            .join(format!("output/input.{target_format}"))
            .is_file(),
        "LIBREOFFICE_FAILED",
        "LibreOffice could not load the file; no output was written.",
    )?;
    let result = workspace.read(&format!("output/input.{target_format}"))?;
    require(
        target_format != "pdf" || result.starts_with(b"%PDF-"),
        "LIBREOFFICE_FAILED",
        "LibreOffice did not create a PDF.",
    )?;
    Ok((
        Some(result),
        json!({"engine":"LibreOffice","available":true,"converted":true,"from":source_format,"to":target_format,"freshProfile":true,"timeoutSeconds":TIMEOUT.as_secs()}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_timeout_uses_private_copies_fresh_profile_and_cleans_up() {
        let mut captured = None;
        let error = convert_with(
            b"{\\rtf1 Hello}",
            "rtf",
            "docx",
            Some("/mock/soffice".into()),
            |command, timeout| {
                assert_eq!(timeout, Duration::from_secs(60));
                assert_eq!(command.get(2), Some(&OsString::from("--headless")));
                assert_eq!(command.get(3), Some(&OsString::from("--norestore")));
                let input = PathBuf::from(command.last().unwrap());
                let folder = input.parent().unwrap();
                assert_eq!(std::fs::read(&input).unwrap(), b"{\\rtf1 Hello}");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    assert_eq!(
                        folder.metadata().unwrap().permissions().mode() & 0o777,
                        0o700
                    );
                }
                let registry =
                    std::fs::read_to_string(folder.join("profile/user/registrymodifications.xcu"))
                        .unwrap();
                assert!(registry.contains("MacroSecurityLevel"));
                assert!(registry.contains("Update/Link"));
                captured = Some(folder.to_owned());
                Err(WorkbenchError::new(
                    "LIBREOFFICE_TIMEOUT",
                    "LibreOffice exceeded the 60-second timeout.",
                ))
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "LIBREOFFICE_TIMEOUT");
        assert!(!captured.unwrap().exists());
    }
    #[test]
    fn exit_zero_requires_output_and_private_output_is_bounded() {
        let error =
            convert_with(b"rtf", "rtf", "docx", Some("mock".into()), |_, _| Ok(0)).unwrap_err();
        assert_eq!(
            error.message,
            "LibreOffice could not load the file; no output was written."
        );
        let (bytes, report) =
            convert_with(b"rtf", "rtf", "pdf", Some("mock".into()), |command, _| {
                let folder = PathBuf::from(command.last().unwrap())
                    .parent()
                    .unwrap()
                    .to_owned();
                std::fs::write(folder.join("output/input.pdf"), b"%PDF-mocked").unwrap();
                Ok(0)
            })
            .unwrap();
        assert_eq!(bytes.unwrap(), b"%PDF-mocked");
        assert_eq!(report["freshProfile"], true);
    }
    #[test]
    fn missing_dependency_does_not_invoke_process() {
        let (bytes, report) = convert_with(b"rtf", "rtf", "docx", None, |_, _| {
            panic!("process must not start")
        })
        .unwrap();
        assert!(bytes.is_none());
        assert_eq!(report["available"], false);
    }
}

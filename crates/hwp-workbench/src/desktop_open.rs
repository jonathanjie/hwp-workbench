//! CLI desktop handoff after no-follow document validation.
use hwp_core::errors::{require, Result, WorkbenchError};
use hwp_core::workspace::Workspace;
use serde_json::{json, Value};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Validated desktop launch request; tests inspect it without starting a GUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub detached: bool,
    pub workspace_root: PathBuf,
}
/// Environment facts needed to choose a packaged app or a source checkout.
pub struct DesktopEnvironment {
    pub macos: bool,
    pub application_directories: Vec<PathBuf>,
    pub project: PathBuf,
}
impl DesktopEnvironment {
    fn current() -> Self {
        let mut directories = vec![PathBuf::from("/Applications")];
        if let Ok(home) = crate::mcp_config::home() {
            directories.push(home.join("Applications"));
        }
        Self {
            macos: cfg!(target_os = "macos"),
            application_directories: directories,
            project: crate::mcp_config::project(),
        }
    }
}
/// Select and execute a handoff through an injected launcher, after path checks.
pub fn open_document_with(
    root: impl AsRef<Path>,
    path: &str,
    environment: &DesktopEnvironment,
    launch: impl FnOnce(&LaunchRequest) -> Result<()>,
) -> Result<Value> {
    let workspace = Workspace::new(root)?;
    let relative = workspace.relative_path(path)?;
    let extension = Path::new(&relative)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    require(
        matches!(extension.as_str(), "hwp" | "hwpx"),
        "OPEN_FORMAT",
        "Desktop review requires an existing .hwp or .hwpx file.",
    )?;
    workspace.read(&relative)?;
    let document = workspace
        .root()
        .join(relative)
        .to_string_lossy()
        .into_owned();
    let packaged = environment.macos
        && environment
            .application_directories
            .iter()
            .any(|folder| folder.join("HWP Workbench.app").is_dir());
    let request = if packaged {
        LaunchRequest {
            program: PathBuf::from("open"),
            args: vec!["-a".to_owned(), "HWP Workbench".to_owned(), document],
            cwd: None,
            detached: false,
            workspace_root: workspace.root().to_owned(),
        }
    } else {
        let electron = environment.project.join("node_modules/.bin/electron");
        let executable = electron
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0);
        require(
            environment.project.join("package.json").is_file()
                && environment.project.join("desktop/main.cjs").is_file()
                && executable,
            "APP_UNAVAILABLE",
            "HWP Workbench.app and a source checkout with installed Electron were not found.",
        )?;
        LaunchRequest {
            program: electron,
            args: vec![
                environment.project.to_string_lossy().into_owned(),
                "--open".to_owned(),
                document,
            ],
            cwd: Some(environment.project.clone()),
            detached: true,
            workspace_root: workspace.root().to_owned(),
        }
    };
    launch(&request)?;
    Ok(json!({"opened":true,"via":if packaged {"app"} else {"electron"}}))
}
fn launch(request: &LaunchRequest) -> Result<()> {
    let failed = |error: String| {
        WorkbenchError::new(
            "OPEN_FAILED",
            format!("Could not launch HWP Workbench: {error}"),
        )
    };
    if request.detached {
        let recipe = source_child_recipe(request)?;
        let mut child = Command::new(recipe.program)
            .args(recipe.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| failed(error.to_string()))?;
        let diagnostic = child
            .stdout
            .take()
            .ok_or_else(|| failed("Desktop child diagnostic pipe is unavailable.".to_owned()))?;
        if let Err(error) = read_child_report(diagnostic) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        std::thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });
        return Ok(());
    }
    let mut command = Command::new(&request.program);
    command.args(&request.args).stdin(Stdio::null());
    if let Some(cwd) = &request.cwd {
        command.current_dir(cwd);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| failed(error.to_string()))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| failed(error.to_string()))?
        {
            if status.success() {
                return Ok(());
            }
            return Err(failed(format!(
                "Command '{}' returned non-zero exit status {}.",
                request.program.display(),
                status.code().unwrap_or(1)
            )));
        }
        if started.elapsed() >= Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failed("Command timed out after 10 seconds".to_owned()));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
/// Open a validated document using the host's installed desktop application.
pub fn open_document(root: impl AsRef<Path>, path: &str) -> Result<Value> {
    open_document_with(root, path, &DesktopEnvironment::current(), launch)
}

fn source_child_recipe(request: &LaunchRequest) -> Result<LaunchRequest> {
    let document = request.args.last().ok_or_else(|| {
        WorkbenchError::new(
            "OPEN_FAILED",
            "Could not launch HWP Workbench: missing document path",
        )
    })?;
    let executable = std::env::current_exe().map_err(open_failed)?;
    Ok(LaunchRequest {
        program: executable,
        args: vec![
            "__desktop-source-open".to_owned(),
            "--root".to_owned(),
            request.workspace_root.to_string_lossy().into_owned(),
            "--path".to_owned(),
            document.clone(),
        ],
        cwd: None,
        detached: false,
        workspace_root: request.workspace_root.clone(),
    })
}

fn open_failed(error: impl std::fmt::Display) -> WorkbenchError {
    WorkbenchError::new(
        "OPEN_FAILED",
        format!("Could not launch HWP Workbench: {error}"),
    )
}

fn source_child_with(
    root: impl AsRef<Path>,
    path: &str,
    environment: &DesktopEnvironment,
    new_session: impl FnOnce() -> Result<()>,
    execute: impl FnOnce(&LaunchRequest) -> Result<()>,
) -> Result<()> {
    // The child repeats path checks and chooses only this checkout's Electron.
    let source_environment = DesktopEnvironment {
        macos: false,
        application_directories: vec![],
        project: environment.project.clone(),
    };
    open_document_with(root, path, &source_environment, |request| {
        new_session()?;
        execute(request)
    })?;
    Ok(())
}

/// Trusted CLI re-exec entry point; no arbitrary program or arguments are accepted.
pub(crate) fn source_child_main(root: impl AsRef<Path>, path: &str) -> Result<()> {
    // Retain the diagnostic pipe while exec replaces stdout with /dev/null.
    // The kernel closes this private descriptor only when exec succeeds.
    let stdout = std::io::stdout();
    let descriptor = match rustix::io::dup(&stdout).map_err(open_failed) {
        Ok(descriptor) => descriptor,
        Err(error) => return write_child_report(Err(error), stdout.lock()),
    };
    if let Err(error) = rustix::io::fcntl_setfd(&descriptor, rustix::io::FdFlags::CLOEXEC) {
        return write_child_report(Err(open_failed(error)), stdout.lock());
    }
    let diagnostic = File::from(descriptor);
    let result = source_child_with(
        root,
        path,
        &DesktopEnvironment::current(),
        || rustix::process::setsid().map(|_| ()).map_err(open_failed),
        |request| {
            let mut command = Command::new(&request.program);
            command
                .args(&request.args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Some(cwd) = &request.cwd {
                command.current_dir(cwd);
            }
            Err(open_failed(command.exec()))
        },
    );
    let result = result.map_err(|error| {
        if error.code == "OPEN_FAILED" {
            error
        } else {
            open_failed(error.message)
        }
    });
    write_child_report(result, diagnostic)
}

fn write_child_report(result: Result<()>, mut output: impl Write) -> Result<()> {
    if let Err(error) = &result {
        serde_json::to_writer(&mut output, &error.as_json()).map_err(open_failed)?;
        output.flush().map_err(open_failed)?;
    }
    result
}

fn read_child_report(mut input: impl Read) -> Result<()> {
    // The internal child sends only one error detail. Bound unexpected output.
    const MAX_DIAGNOSTIC: usize = 64 * 1024;
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take((MAX_DIAGNOSTIC + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(open_failed)?;
    require(
        bytes.len() <= MAX_DIAGNOSTIC,
        "OPEN_FAILED",
        "Could not launch HWP Workbench: desktop diagnostic exceeds limit.",
    )?;
    if bytes.is_empty() {
        return Ok(());
    }
    let report: Value = serde_json::from_slice(&bytes).map_err(open_failed)?;
    let message = report
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Could not launch HWP Workbench: invalid desktop diagnostic.");
    Err(WorkbenchError::new("OPEN_FAILED", message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn source_child_creates_session_before_fixed_electron_exec() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let project = root.path().join("checkout");
        let electron = project.join("node_modules/.bin/electron");
        std::fs::create_dir_all(electron.parent().unwrap()).unwrap();
        std::fs::create_dir(project.join("desktop")).unwrap();
        std::fs::write(project.join("package.json"), "{}").unwrap();
        std::fs::write(project.join("desktop/main.cjs"), "// test").unwrap();
        std::fs::write(&electron, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&electron, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(root.path().join("form.hwpx"), b"document").unwrap();
        let environment = DesktopEnvironment {
            macos: true,
            application_directories: vec![],
            project: project.clone(),
        };
        let calls = RefCell::new(Vec::new());
        source_child_with(
            root.path(),
            "form.hwpx",
            &environment,
            || {
                calls.borrow_mut().push("setsid");
                Ok(())
            },
            |request| {
                assert_eq!(*calls.borrow(), vec!["setsid"]);
                assert_eq!(request.program, electron);
                assert_eq!(
                    request.args,
                    vec![
                        project.to_string_lossy().into_owned(),
                        "--open".to_owned(),
                        root.path().join("form.hwpx").to_string_lossy().into_owned()
                    ]
                );
                let recipe = source_child_recipe(request).unwrap();
                assert_eq!(recipe.program, std::env::current_exe().unwrap());
                assert_eq!(
                    recipe.args,
                    vec![
                        "__desktop-source-open".to_owned(),
                        "--root".to_owned(),
                        root.path().to_string_lossy().into_owned(),
                        "--path".to_owned(),
                        root.path().join("form.hwpx").to_string_lossy().into_owned(),
                    ]
                );
                assert!(
                    !recipe.detached,
                    "the re-exec child must remain eligible for setsid"
                );
                crate::cli::parser()
                    .try_get_matches_from(
                        std::iter::once("hwp-workbench")
                            .chain(recipe.args.iter().map(String::as_str)),
                    )
                    .unwrap();
                calls.borrow_mut().push("exec");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*calls.borrow(), vec!["setsid", "exec"]);
        let mut diagnostic = Vec::new();
        let result = source_child_with(
            root.path(),
            "form.hwpx",
            &environment,
            || Ok(()),
            |_| Err(open_failed("injected executable format failure")),
        );
        assert_eq!(
            write_child_report(result, &mut diagnostic)
                .unwrap_err()
                .code,
            "OPEN_FAILED"
        );
        let error = read_child_report(diagnostic.as_slice()).unwrap_err();
        assert_eq!(error.code, "OPEN_FAILED");
        assert_eq!(
            error.message,
            "Could not launch HWP Workbench: injected executable format failure"
        );
        assert!(read_child_report(std::io::empty()).is_ok());
    }

    #[test]
    fn source_child_revalidates_paths_before_session_or_exec() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        std::fs::write(root.path().join("form.hwpx"), b"document").unwrap();
        std::os::unix::fs::symlink(
            root.path().join("form.hwpx"),
            root.path().join("linked.hwpx"),
        )
        .unwrap();
        let environment = DesktopEnvironment {
            macos: true,
            application_directories: vec![],
            project: root.path().join("checkout"),
        };
        for path in [
            "../outside.hwpx",
            "/outside.hwpx",
            "linked.hwpx",
            "missing.hwpx",
            "wrong.pdf",
        ] {
            assert!(source_child_with(
                root.path(),
                path,
                &environment,
                || panic!("invalid child path must not create a session"),
                |_| panic!("invalid child path must not execute Electron"),
            )
            .is_err());
        }
    }
}

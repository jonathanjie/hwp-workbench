//! Shared desktop engine boundary; subprocess details remain in PythonEngine.
use crate::{Error, Result};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub type EngineError = Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolAction {
    Status,
    Install { target: String },
    McpConfig { client: String, root: PathBuf },
    BuildBundle,
}

pub trait Engine: Send + Sync {
    fn call(&self, root: &Path, method: &str, args: Value) -> Result<Value>;
    fn call_cancelled(
        &self,
        root: &Path,
        method: &str,
        args: Value,
        cancel: Arc<AtomicBool>,
    ) -> Result<Value> {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::new("Preview dropped"));
        }
        self.call(root, method, args)
    }
    fn agent_link(&self, _root: &Path, _name: &str, _action: &str, _args: Value) -> Result<Value> {
        Err(Error::new("Agent link unavailable"))
    }
    fn tool(&self, _action: ToolAction) -> Result<String> {
        Err(Error::new("Agent tools unavailable"))
    }
}

pub struct PythonEngine {
    source_root: PathBuf,
    python: Mutex<Option<PathBuf>>,
}

// Match the Electron request and process budgets. Preview responses include images.
const MAX_REQUEST: usize = 4 * 1024 * 1024;
const MAX_LINK_REQUEST: usize = 2 * 1024 * 1024;
const MAX_OUTPUT: usize = 32 * 1024 * 1024;
const MAX_PREVIEW_OUTPUT: usize = 90 * 1024 * 1024;
const MAX_STDERR: usize = 65536;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);

impl PythonEngine {
    pub fn new(source_root: PathBuf) -> Self {
        Self {
            source_root,
            python: Mutex::new(None),
        }
    }

    fn command(&self, executable: &Path, arguments: &[OsString]) -> Command {
        let mut command = Command::new(executable);
        command
            .args(arguments)
            .current_dir(&self.source_root)
            .env("PYTHONPATH", self.source_root.join("engine"))
            .env("PYTHONUTF8", "1")
            .env("PYTHONDONTWRITEBYTECODE", "1");
        command
    }

    pub fn python_executable(&self) -> Result<PathBuf> {
        let mut selected = self
            .python
            .lock()
            .map_err(|_| Error::new("Python unavailable"))?;
        if let Some(executable) = selected.as_ref() {
            return Ok(executable.clone());
        }
        let override_python = std::env::var_os("HWP_WORKBENCH_PYTHON")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        if let Some(executable) = configured_python(override_python, &self.source_root) {
            *selected = Some(executable.clone());
            return Ok(executable);
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let candidates = [
            PathBuf::from("/opt/homebrew/bin/python3"),
            PathBuf::from("/usr/local/bin/python3"),
            home.join(".local/bin/python3"),
            home.join(".pyenv/shims/python3"),
            PathBuf::from("/Library/Frameworks/Python.framework/Versions/Current/bin/python3"),
            PathBuf::from("/opt/miniconda3/bin/python3"),
            PathBuf::from("/usr/bin/python3"),
            PathBuf::from("python3"),
        ];
        let executable = select_candidate(candidates, |candidate| {
            let arguments = [
                OsString::from("-c"),
                OsString::from("import sys; print(sys.version_info[0], sys.version_info[1])"),
            ];
            run_process(
                self.command(candidate, &arguments),
                Vec::new(),
                4096,
                Duration::from_secs(10),
                None,
            )
            .ok()
        });
        if let Some(executable) = executable {
            *selected = Some(executable.clone());
            return Ok(executable);
        }
        Err(Error::new(
            "HWP Workbench needs Python 3.11 or newer / Python 3.11 이상이 필요합니다",
        ))
    }

    fn run(
        &self,
        arguments: Vec<OsString>,
        input: Vec<u8>,
        max_bytes: usize,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<Vec<u8>> {
        if cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            return Err(Error::new("Preview dropped"));
        }
        let executable = self.python_executable()?;
        run_process(
            self.command(&executable, &arguments),
            input,
            max_bytes,
            PROCESS_TIMEOUT,
            cancel,
        )
    }

    fn rpc(
        &self,
        root: &Path,
        method: &str,
        args: Value,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<Value> {
        let input = serde_json::to_vec(&json!({"method":method,"args":args}))?;
        if input.len() > MAX_REQUEST {
            return Err(Error::new("Request too large"));
        }
        let arguments = vec![
            "-m".into(),
            "hwp_workbench".into(),
            "rpc".into(),
            "--root".into(),
            root.as_os_str().to_owned(),
        ];
        let output = self.run(
            arguments,
            input,
            if method == "preview" {
                MAX_PREVIEW_OUTPUT
            } else {
                MAX_OUTPUT
            },
            cancel,
        )?;
        serde_json::from_slice(&output).map_err(|_| Error::new("Invalid engine response"))
    }
}

impl Engine for PythonEngine {
    fn call(&self, root: &Path, method: &str, args: Value) -> Result<Value> {
        self.rpc(root, method, args, None)
    }

    fn call_cancelled(
        &self,
        root: &Path,
        method: &str,
        args: Value,
        cancel: Arc<AtomicBool>,
    ) -> Result<Value> {
        self.rpc(root, method, args, Some(cancel))
    }

    fn agent_link(&self, root: &Path, name: &str, action: &str, args: Value) -> Result<Value> {
        if !matches!(action, "read" | "session" | "outcome") {
            return Err(Error::new("Unknown link action."));
        }
        let input = serde_json::to_vec(&args)?;
        if input.len() > MAX_LINK_REQUEST {
            return Err(Error::new("Link request too large"));
        }
        let arguments = vec![
            self.source_root
                .join("desktop/agent-files.py")
                .into_os_string(),
            root.as_os_str().to_owned(),
            action.into(),
            name.into(),
        ];
        let output = self.run(arguments, input, MAX_OUTPUT, None)?;
        serde_json::from_slice(&output).map_err(|_| Error::new("Invalid engine response"))
    }

    fn tool(&self, action: ToolAction) -> Result<String> {
        let arguments: Vec<OsString> = match action {
            ToolAction::Status => vec![
                "-m".into(),
                "hwp_workbench".into(),
                "skill".into(),
                "status".into(),
            ],
            ToolAction::Install { target } => {
                if !matches!(target.as_str(), "claude" | "codex") {
                    return Err(Error::new("Invalid skill target"));
                }
                vec![
                    "-m".into(),
                    "hwp_workbench".into(),
                    "skill".into(),
                    "install".into(),
                    "--target".into(),
                    target.into(),
                ]
            }
            ToolAction::McpConfig { client, root } => {
                if !matches!(client.as_str(), "claude-code" | "codex") {
                    return Err(Error::new("Invalid MCP client"));
                }
                vec![
                    "-m".into(),
                    "hwp_workbench".into(),
                    "mcp-config".into(),
                    "--client".into(),
                    client.into(),
                    "--root".into(),
                    root.into_os_string(),
                ]
            }
            ToolAction::BuildBundle => vec![self
                .source_root
                .join("scripts/build-mcpb.py")
                .into_os_string()],
        };
        String::from_utf8(self.run(arguments, Vec::new(), MAX_OUTPUT, None)?)
            .map_err(|_| Error::new("Invalid engine response"))
    }
}

fn configured_python(override_python: Option<PathBuf>, source_root: &Path) -> Option<PathBuf> {
    // Packaged resources carry Python next to the application source tree.
    override_python.or_else(|| {
        source_root
            .parent()
            .map(|parent| parent.join("python/bin/python3"))
            .filter(|candidate| candidate.is_file())
    })
}

fn select_candidate(
    candidates: impl IntoIterator<Item = PathBuf>,
    mut probe: impl FnMut(&Path) -> Option<Vec<u8>>,
) -> Option<PathBuf> {
    candidates.into_iter().find(|candidate| {
        let Some(bytes) = probe(candidate) else { return false; };
        let Ok(version) = std::str::from_utf8(&bytes) else { return false; };
        let parts: Vec<_> = version.split_whitespace().collect();
        let [major, minor] = parts.as_slice() else { return false; };
        if !major.bytes().chain(minor.bytes()).all(|byte| byte.is_ascii_digit()) { return false; }
        matches!((major.parse::<u32>(), minor.parse::<u32>()), (Ok(major), Ok(minor)) if major > 3 || (major == 3 && minor >= 11))
    })
}

enum ProcessOutput {
    Data { stderr: bool, bytes: Vec<u8> },
    Finished,
}

fn drain_pipe(mut pipe: impl Read, stderr: bool, sender: SyncSender<ProcessOutput>) {
    let mut buffer = [0; 8192];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(size) => {
                if sender
                    .send(ProcessOutput::Data {
                        stderr,
                        bytes: buffer.get(..size).unwrap_or_default().to_vec(),
                    })
                    .is_err()
                {
                    return;
                }
            }
        }
    }
    let _ = sender.send(ProcessOutput::Finished);
}

fn run_process(
    mut command: Command,
    input: Vec<u8>,
    max_bytes: usize,
    timeout: Duration,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<Vec<u8>> {
    use std::os::unix::process::CommandExt;
    // Engine children can launch fixed native bridges. A separate process group
    // lets cancellation close inherited pipe handles as well as the interpreter.
    command.process_group(0);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| Error::new("Python unavailable"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::new("Python unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("Python unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::new("Python unavailable"))?;
    let writer = thread::spawn(move || {
        let mut stdin = stdin;
        let _ = stdin.write_all(&input);
    });
    // Bounded channels prevent an untrusted engine from outrunning the output cap.
    let (sender, receiver) = mpsc::sync_channel(8);
    let stdout_sender = sender.clone();
    let stdout_reader = thread::spawn(move || drain_pipe(stdout, false, stdout_sender));
    let stderr_reader = thread::spawn(move || drain_pipe(stderr, true, sender));
    let started = Instant::now();
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let mut finished = 0;
    let mut process_exited = false;
    let result = (|| loop {
        if cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            return Err(Error::new("Preview dropped"));
        }
        if started.elapsed() >= timeout {
            return Err(Error::new("Processing timed out"));
        }
        match receiver.recv_timeout(Duration::from_millis(5)) {
            Ok(ProcessOutput::Data {
                stderr: true,
                bytes,
            }) => {
                let remaining = MAX_STDERR.saturating_sub(errors.len());
                errors.extend(bytes.into_iter().take(remaining));
            }
            Ok(ProcessOutput::Data {
                stderr: false,
                bytes,
            }) => {
                if bytes.len() > max_bytes.saturating_sub(output.len()) {
                    return Err(Error::new("Response too large"));
                }
                output.extend(bytes);
            }
            Ok(ProcessOutput::Finished) => finished += 1,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                finished = 2;
                thread::sleep(Duration::from_millis(5));
            }
        }
        if finished == 2 {
            if let Some(status) = child.try_wait()? {
                process_exited = true;
                if status.success() {
                    return Ok(output);
                }
                if let Ok(value) = serde_json::from_slice::<Value>(&errors) {
                    if let Some(message) = value
                        .get("error")
                        .and_then(|error| error.get("message"))
                        .and_then(Value::as_str)
                    {
                        let code = value
                            .get("error")
                            .and_then(|error| error.get("code"))
                            .and_then(Value::as_str);
                        return Err(match code {
                            Some(code) => Error::coded(code, message),
                            None => Error::new(message),
                        });
                    }
                }
                return Err(Error::new("Engine failed"));
            }
        }
    })();
    if result.is_err() && !process_exited {
        let group = rustix::process::Pid::from_child(&child);
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        let _ = child.kill();
    }
    let _ = child.wait();
    drop(receiver);
    let _ = writer.join();
    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_does_not_wait_for_descendant_pipe_handles() {
        let mut command = Command::new("/usr/bin/python3");
        command.args(["-c", "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c','import time; time.sleep(1)']); time.sleep(1)"]);
        let started = Instant::now();
        let error =
            run_process(command, Vec::new(), 1024, Duration::from_millis(100), None).unwrap_err();
        assert_eq!(error.message, "Processing timed out");
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "timeout waited for descendants: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn process_output_is_bounded_and_json_errors_keep_codes() {
        let mut oversized = Command::new("/usr/bin/python3");
        oversized.args(["-c", "print('x' * 4096)"]);
        assert_eq!(
            run_process(oversized, Vec::new(), 100, Duration::from_secs(2), None)
                .unwrap_err()
                .message,
            "Response too large"
        );
        let mut failing = Command::new("/usr/bin/python3");
        failing.args(["-c", "import sys; print('{\"error\":{\"code\":\"SOURCE_CHANGED\",\"message\":\"Source changed\"}}',file=sys.stderr); sys.exit(2)"]);
        let error =
            run_process(failing, Vec::new(), 100, Duration::from_secs(2), None).unwrap_err();
        assert_eq!(error.code.as_deref(), Some("SOURCE_CHANGED"));
        assert_eq!(error.message, "Source changed");
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;

    #[test]
    fn packaged_and_development_paths_keep_spaces_and_use_the_correct_root() {
        let source = PathBuf::from("/Applications/HWP Workbench.app/Contents/Resources/app");
        let engine = PythonEngine::new(source.clone());
        let command = engine.command(
            Path::new("/python"),
            &[source.join("scripts/hwp-workbench").into_os_string()],
        );
        assert_eq!(command.get_current_dir(), Some(source.as_path()));
        assert_eq!(
            command.get_args().next(),
            Some(source.join("scripts/hwp-workbench").as_os_str())
        );
        let checkout = PathBuf::from("/work/checkout");
        assert_eq!(
            PythonEngine::new(checkout.clone())
                .command(Path::new("/python"), &[])
                .get_current_dir(),
            Some(checkout.as_path())
        );
    }

    #[test]
    fn development_skips_python_39_failures_malformed_output_and_timed_out_probes() {
        let candidates = ["first", "second", "third", "fourth", "fifth"].map(PathBuf::from);
        let mut responses = [
            Some(b"3 9\n".to_vec()),
            None,
            Some(b"not a version".to_vec()),
            None,
            Some(b"3 13\n".to_vec()),
        ]
        .into_iter();
        let mut calls = Vec::new();
        let selected = select_candidate(candidates, |candidate| {
            calls.push(candidate.to_owned());
            responses.next().flatten()
        });
        assert_eq!(selected, Some(PathBuf::from("fifth")));
        assert_eq!(calls.len(), 5);
    }

    #[test]
    fn python_311_is_accepted_unavailable_interpreters_return_none() {
        let candidates = [PathBuf::from("/opt/homebrew/bin/python3")];
        assert_eq!(
            select_candidate(candidates.clone(), |_| Some(b"3 11".to_vec())),
            candidates.first().cloned()
        );
        assert_eq!(
            select_candidate(candidates, |_| Some(b"3 9".to_vec())),
            None
        );
    }

    #[test]
    fn engine_environment_has_fixed_python_settings() {
        let engine = PythonEngine::new(PathBuf::from("/source tree"));
        let command = engine.command(Path::new("/python"), &[]);
        let environment: std::collections::HashMap<_, _> = command.get_envs().collect();
        assert_eq!(
            environment.get(std::ffi::OsStr::new("PYTHONPATH")),
            Some(&Some(std::ffi::OsStr::new("/source tree/engine")))
        );
        assert_eq!(
            environment.get(std::ffi::OsStr::new("PYTHONUTF8")),
            Some(&Some(std::ffi::OsStr::new("1")))
        );
        assert_eq!(
            environment.get(std::ffi::OsStr::new("PYTHONDONTWRITEBYTECODE")),
            Some(&Some(std::ffi::OsStr::new("1")))
        );
    }

    #[test]
    fn an_explicitly_selected_node_bridge_is_retained() {
        let engine = PythonEngine::new(PathBuf::from("/source"));
        let command = engine.command(Path::new("/python"), &[]);
        // Command inherits both variables without replacing the selected bridge.
        assert!(!command
            .get_envs()
            .any(|(name, _)| name == "HWP_WORKBENCH_NODE" || name == "ELECTRON_RUN_AS_NODE"));
    }
}

#[cfg(test)]
mod configured_runtime_tests {
    use super::*;
    #[test]
    fn packaged_python_is_bundled_an_explicit_override_takes_precedence() {
        let scratch = tempfile::tempdir().unwrap();
        let resources = scratch.path().join("HWP Workbench.app/Contents/Resources");
        let bundled = resources.join("python/bin/python3");
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        std::fs::write(&bundled, b"bundled executable").unwrap();
        let source = resources.join("app");
        assert_eq!(configured_python(None, &source), Some(bundled));
        let custom = PathBuf::from("/custom/python");
        assert_eq!(
            configured_python(Some(custom.clone()), &source),
            Some(custom)
        );
    }
}

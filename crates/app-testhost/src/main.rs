//! Development-only host. Never linked by the Tauri application or release packaging.
use app_core::{lock, queue::CommandQueue, Backend, Dialogs, Error, Events, Result, Shell};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

// Match the desktop's maximum IPC request size. Static WASM uses the file budget.
const MAX_REQUEST: u64 = 4 * 1024 * 1024;
const MAX_ASSET: usize = 64 * 1024 * 1024;
// Bound event buffering if a browser stops reading during a long preview.
const MAX_EVENTS: usize = 256;

#[derive(Default)]
struct ScriptState {
    opens: VecDeque<Option<PathBuf>>,
    saves: VecDeque<Option<PathBuf>>,
    answers: VecDeque<usize>,
    messages: Vec<Value>,
    revealed: Vec<PathBuf>,
    copied: Vec<String>,
}
#[derive(Default)]
struct Platform {
    state: Mutex<ScriptState>,
    listeners: Mutex<Vec<mpsc::SyncSender<String>>>,
}
impl Platform {
    fn subscribe(&self) -> Result<mpsc::Receiver<String>> {
        let (sender, receiver) = mpsc::sync_channel(MAX_EVENTS);
        lock(&self.listeners)?.push(sender);
        Ok(receiver)
    }
    fn snapshot(&self) -> Result<Value> {
        let state = lock(&self.state)?;
        Ok(json!({"messages":state.messages,"revealed":state.revealed,"copied":state.copied}))
    }
}
impl Dialogs for Platform {
    fn open_file(&self) -> Result<Option<PathBuf>> {
        Ok(lock(&self.state)?.opens.pop_front().flatten())
    }
    fn save_file(&self, _default: &Path, _extensions: &[String]) -> Result<Option<PathBuf>> {
        Ok(lock(&self.state)?.saves.pop_front().flatten())
    }
    fn message(&self, message: &str, buttons: &[&str], default: usize) -> Result<usize> {
        let mut state = lock(&self.state)?;
        state
            .messages
            .push(json!({"message":message,"buttons":buttons,"default":default}));
        let answer = state.answers.pop_front().unwrap_or(default);
        if answer >= buttons.len() {
            return Err(Error::new("Invalid scripted message answer"));
        }
        Ok(answer)
    }
}
impl Shell for Platform {
    fn reveal(&self, file: &Path) -> Result<()> {
        lock(&self.state)?.revealed.push(file.to_owned());
        Ok(())
    }
    fn copy(&self, text: &str) -> Result<()> {
        lock(&self.state)?.copied.push(text.to_owned());
        Ok(())
    }
}
impl Events for Platform {
    fn emit(&self, event: &str, payload: Value) -> Result<()> {
        let frame = format!(
            "event: {event}\ndata: {}\n\n",
            serde_json::to_string(&payload)?
        );
        // A full buffer disconnects the subscriber rather than silently losing an event.
        lock(&self.listeners)?.retain(|sender| sender.try_send(frame.clone()).is_ok());
        Ok(())
    }
}

fn route<'a>(url: &'a str, token: &str) -> Result<&'a str> {
    let path = url.split('?').next().unwrap_or_default();
    let route = path
        .strip_prefix(&format!("/{token}/"))
        .ok_or_else(|| Error::new("Unauthorized test host request"))?;
    if route.is_empty()
        || route.contains(['%', '\\', '\0'])
        || route
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(Error::new("Invalid test host path"));
    }
    Ok(route)
}
fn asset(root: &Path, relative: &str) -> Result<(Vec<u8>, &'static str)> {
    let mime = match Path::new(relative)
        .extension()
        .and_then(|part| part.to_str())
    {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("wasm") => "application/wasm",
        _ => return Err(Error::new("Unsupported asset type")),
    };
    let mut file = root.to_owned();
    for part in Path::new(relative).components() {
        let Component::Normal(name) = part else {
            return Err(Error::new("Invalid asset path"));
        };
        file.push(name);
        if std::fs::symlink_metadata(&file)?.file_type().is_symlink() {
            return Err(Error::new("Refuse asset symlink"));
        }
    }
    // Use the backend's bounded, no-follow reader after validating the asset allowlist.
    Ok((app_core::files::read_bounded(&file, MAX_ASSET)?, mime))
}
fn read_json(request: &mut Request) -> Result<Value> {
    if request
        .body_length()
        .is_some_and(|size| size as u64 > MAX_REQUEST)
    {
        return Err(Error::new("Request too large"));
    }
    let mut bytes = Vec::new();
    request
        .as_reader()
        .take(MAX_REQUEST + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_REQUEST {
        return Err(Error::new("Request too large"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn header(name: &str, value: &str) -> Result<Header> {
    Header::from_bytes(name, value).map_err(|_| Error::new("Invalid response header"))
}
fn respond(request: Request, status: u16, bytes: Vec<u8>, mime: &str) -> Result<()> {
    let response = Response::from_data(bytes)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", mime)?)
        .with_header(header("Cache-Control", "no-store")?)
        .with_header(header("X-Content-Type-Options", "nosniff")?)
        .with_header(header("Referrer-Policy", "no-referrer")?);
    request.respond(response)?;
    Ok(())
}
fn respond_json(request: Request, result: Result<Value>) -> Result<()> {
    let (status, value) = match result {
        Ok(value) => (200, value),
        Err(error) => (400, serde_json::to_value(error)?),
    };
    respond(
        request,
        status,
        serde_json::to_vec(&value)?,
        "application/json",
    )
}
fn stream(request: Request, receiver: mpsc::Receiver<String>) -> Result<()> {
    // Write and flush each frame directly; tiny_http's response copier buffers chunks.
    let mut writer = request.into_writer();
    writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n: ready\n\n")?;
    writer.flush()?;
    loop {
        let frame = match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(frame) => frame,
            Err(mpsc::RecvTimeoutError::Timeout) => ": keepalive\n\n".into(),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };
        writer.write_all(frame.as_bytes())?;
        writer.flush()?;
    }
}
fn path_answer(args: &Value) -> Result<Option<PathBuf>> {
    match args.get("path") {
        Some(Value::Null) => Ok(None),
        Some(Value::String(path)) if Path::new(path).is_absolute() => Ok(Some(path.into())),
        _ => Err(Error::new("Invalid scripted dialog path")),
    }
}
fn wait_response(receiver: app_core::queue::Response) -> Result<Value> {
    receiver
        .recv()
        .map_err(|_| Error::new("Desktop queue unavailable"))?
}
struct Host {
    backend: Arc<Backend>,
    queue: CommandQueue,
    platform: Arc<Platform>,
    assets: PathBuf,
    token: String,
}
impl Host {
    fn test(&self, action: &str, args: Value) -> Result<Value> {
        match action {
            "dialog/open" => lock(&self.platform.state)?
                .opens
                .push_back(path_answer(&args)?),
            "dialog/save" => lock(&self.platform.state)?
                .saves
                .push_back(path_answer(&args)?),
            "dialog/message" => {
                let answer = args
                    .get("answer")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| Error::new("Invalid scripted message answer"))?;
                lock(&self.platform.state)?.answers.push_back(answer);
            }
            "state" => return self.platform.snapshot(),
            "menu" => {
                let command = args
                    .get("command")
                    .and_then(Value::as_str)
                    .filter(|name| {
                        [
                            "open",
                            "save",
                            "saveCopy",
                            "language",
                            "palette",
                            "find",
                            "replace",
                            "shortcuts",
                            "undo",
                            "redo",
                        ]
                        .contains(name)
                    })
                    .ok_or_else(|| Error::new("Invalid menu command"))?;
                self.platform
                    .emit("app:command", Value::String(command.into()))?;
            }
            "handoff" => {
                let path = path_answer(&args)?.ok_or_else(|| Error::new("Invalid handoff path"))?;
                return wait_response(self.queue.open(path)?);
            }
            "drop" => {
                let paths = args
                    .get("paths")
                    .and_then(Value::as_array)
                    .ok_or_else(|| Error::new("Invalid drop paths"))?
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(PathBuf::from)
                            .filter(|path| path.is_absolute())
                            .ok_or_else(|| Error::new("Invalid drop paths"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                self.backend.grant_drop(paths.clone())?;
                self.platform
                    .emit("tauri://drag-drop", json!({"paths":paths}))?;
            }
            _ => return Err(Error::new("Unknown test command")),
        }
        Ok(Value::Null)
    }
    fn request(&self, mut request: Request) -> Result<()> {
        let relative = match route(request.url(), &self.token) {
            Ok(relative) => relative.to_owned(),
            Err(error) => {
                return respond(
                    request,
                    403,
                    serde_json::to_vec(&error)?,
                    "application/json",
                )
            }
        };
        if request.method() == &Method::Get && relative == "events" {
            let receiver = self.platform.subscribe()?;
            std::thread::spawn(move || {
                let _ = stream(request, receiver);
            });
            return Ok(());
        }
        if request.method() == &Method::Post {
            let args = match read_json(&mut request) {
                Ok(args) => args,
                Err(error) => return respond_json(request, Err(error)),
            };
            if let Some(command) = relative.strip_prefix("invoke/") {
                // Reserve command order on receipt, as the Tauri dispatcher does.
                let prepared = if command == "document_preview" {
                    self.backend
                        .prepare_preview(&args)
                        .map(|preview| Work::Preview(Box::new(preview)))
                } else {
                    self.queue.invoke(command, args).map(Work::Queued)
                };
                eprintln!("Invoke: {command}");
                let command = command.to_owned();
                let backend = self.backend.clone();
                std::thread::spawn(move || {
                    let result = prepared.and_then(|work| match work {
                        Work::Preview(preview) => backend.run_preview(*preview),
                        Work::Queued(receiver) => wait_response(receiver),
                    });
                    if let Err(error) = &result {
                        eprintln!("Invoke failed: {command}: {error}");
                    }
                    let _ = respond_json(request, result);
                });
                return Ok(());
            }
            if let Some(action) = relative.strip_prefix("test/") {
                eprintln!("Test control: {action}");
                return respond_json(request, self.test(action, args));
            }
        }
        if request.method() == &Method::Get {
            let result = asset(&self.assets, &relative).and_then(|(bytes, mime)| {
                if relative == "index.html" {
                    self.backend.renderer_loading()?;
                }
                Ok((bytes, mime))
            });
            return match result {
                Ok((bytes, mime)) => respond(request, 200, bytes, mime),
                Err(error) => respond(
                    request,
                    404,
                    serde_json::to_vec(&error)?,
                    "application/json",
                ),
            };
        }
        respond(
            request,
            404,
            b"Unknown test host endpoint".to_vec(),
            "text/plain",
        )
    }
}
enum Work {
    Preview(Box<app_core::PreviewRequest>),
    Queued(app_core::queue::Response),
}
fn run() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let source_root = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("Usage: app-testhost <source-root> <profile-root>"))?
        .canonicalize()?;
    let profile = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("Missing profile root"))?;
    if arguments.next().is_some() {
        return Err(Error::new("Unexpected test host argument"));
    }
    let platform = Arc::new(Platform::default());
    let engine = app_core::engine::desktop_engine(source_root.clone());
    let backend = Arc::new(Backend::new(
        engine,
        platform.clone(),
        platform.clone(),
        platform.clone(),
        source_root.clone(),
        profile,
    )?);
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|error| Error::new(error.to_string()))?;
    let token = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let server = Server::http("127.0.0.1:0").map_err(|error| Error::new(error.to_string()))?;
    let host = Host {
        queue: CommandQueue::new(backend.clone()),
        backend,
        platform,
        assets: source_root.join("app/dist").canonicalize()?,
        token,
    };
    println!(
        "{}",
        json!({"url":format!("http://{}/{}/",server.server_addr(),host.token),"token":host.token})
    );
    std::io::stdout().flush()?;
    for request in server.incoming_requests() {
        // Never log URLs: the token is part of every authenticated path.
        eprintln!("{} request", request.method());
        if let Err(error) = host.request(request) {
            eprintln!("Host error: {error}");
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests;

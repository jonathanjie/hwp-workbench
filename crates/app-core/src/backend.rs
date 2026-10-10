//! Document sessions, narrow command validation and ordered native operations.
use crate::{
    engine::Engine, files, inbox, lock, preview, recent, tools, Dialogs, Error, Events, Result,
    Session, Shell,
};
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    current: Option<Session>,
    staged: Value,
    selection: Value,
    editing: bool,
    reviewed: HashSet<PathBuf>,
    output: Option<PathBuf>,
    previous: Option<PathBuf>,
    drops: HashSet<PathBuf>,
    opens: VecDeque<PathBuf>,
    ready: bool,
    handoff_active: bool,
}
pub struct PreviewRequest {
    current: Session,
    patch: Value,
    token: preview::PreviewToken,
}

pub struct Backend {
    engine: Arc<dyn Engine>,
    dialogs: Arc<dyn Dialogs>,
    events: Arc<dyn Events>,
    shell: Arc<dyn Shell>,
    source_root: PathBuf,
    profile: PathBuf,
    state: Mutex<State>,
    queue: Mutex<()>,
    previews: preview::LatestPreview,
    recents: recent::RecentStore,
    inbox: inbox::AgentInbox,
}

// Limits bound renderer-controlled patches before they reach the engine.
const MAX_PATCH_BYTES: usize = 2 * 1024 * 1024;
const MAX_OPERATIONS: usize = 1000;
pub fn validate_patch(patch: &Value, current_hash: Option<&str>) -> Result<()> {
    let valid = current_hash.is_some()
        && patch.get("schemaVersion").and_then(Value::as_u64) == Some(1)
        && patch.get("baseSha256").and_then(Value::as_str) == current_hash
        && patch
            .get("operations")
            .and_then(Value::as_array)
            .is_some_and(|ops| ops.len() <= MAX_OPERATIONS);
    if !valid {
        return Err(Error::new("Invalid or stale patch"));
    }
    if serde_json::to_vec(patch)?.len() > MAX_PATCH_BYTES {
        return Err(Error::new("Patch too large"));
    }
    Ok(())
}
pub fn open_argument(arguments: &[String]) -> Option<PathBuf> {
    let mut flags = arguments
        .iter()
        .enumerate()
        .filter(|(_, arg)| *arg == "--open");
    let (index, _) = flags.next()?;
    if flags.next().is_some() {
        return None;
    }
    let next = index.checked_add(1)?;
    let file = PathBuf::from(arguments.get(next)?);
    file.is_absolute().then_some(file)
}
fn required_text<'a>(args: &'a Value, name: &str, error: &str) -> Result<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new(error))
}
fn argument(args: &Value, name: &str) -> Value {
    args.get(name).cloned().unwrap_or(Value::Null)
}
fn boolean(args: &Value, name: &str, error: &str) -> Result<bool> {
    args.get(name)
        .and_then(Value::as_bool)
        .ok_or_else(|| Error::new(error))
}
fn basename(file: &Path) -> Result<String> {
    file.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| Error::new("Unsupported document"))
}
impl Backend {
    pub fn new(
        engine: Arc<dyn Engine>,
        dialogs: Arc<dyn Dialogs>,
        events: Arc<dyn Events>,
        shell: Arc<dyn Shell>,
        source_root: PathBuf,
        profile: PathBuf,
    ) -> Result<Self> {
        files::ensure_private_directory(&profile)?;
        let inbox = inbox::AgentInbox::new(
            engine.clone(),
            events.clone(),
            profile.join("agent-links.json"),
        );
        Ok(Self {
            engine,
            dialogs,
            events,
            shell,
            source_root,
            profile: profile.clone(),
            state: Mutex::new(State::default()),
            queue: Mutex::new(()),
            previews: preview::LatestPreview::default(),
            recents: recent::RecentStore::new(profile),
            inbox,
        })
    }
    pub fn current(&self) -> Result<Session> {
        lock(&self.state)?
            .current
            .clone()
            .ok_or_else(|| Error::new("No document"))
    }
    pub fn has_unsaved(&self) -> Result<bool> {
        let state = lock(&self.state)?;
        Ok(state.editing
            || state
                .staged
                .get("operations")
                .and_then(Value::as_array)
                .is_some_and(|ops| !ops.is_empty()))
    }
    pub fn confirm_close(&self) -> Result<bool> {
        let _ordered = lock(&self.queue)?;
        if !self.has_unsaved()? {
            return Ok(true);
        }
        self.dialogs.message(
            "Unsaved text / 저장하지 않은 텍스트\nStaged draft retained · unstaged text unsaved / 적용된 초안 보관 · 미적용 텍스트 미저장",
            &["Keep editing / 계속 편집", "Close / 닫기"], 0,
        ).map(|answer| answer == 1)
    }
    pub fn stop(&self) {
        self.previews.cancel();
        self.inbox.stop();
    }
    pub fn grant_drop(&self, paths: Vec<PathBuf>) -> Result<()> {
        let mut state = lock(&self.state)?;
        state.drops.clear();
        // The preload accepts exactly one OS-dropped document.
        if paths.len() == 1 {
            state.drops.extend(paths);
        }
        Ok(())
    }
    pub fn queue_open(&self, file: PathBuf) -> Result<()> {
        lock(&self.state)?.opens.push_back(file);
        self.events.focus()?;
        let _ordered = lock(&self.queue)?;
        self.flush_opens()
    }
    pub fn renderer_loading(&self) -> Result<()> {
        lock(&self.state)?.ready = false;
        Ok(())
    }
    fn flush_opens(&self) -> Result<()> {
        loop {
            let file = {
                let mut state = lock(&self.state)?;
                if !state.ready || state.handoff_active {
                    return Ok(());
                }
                let Some(file) = state.opens.pop_front() else {
                    return Ok(());
                };
                state.handoff_active = true;
                file
            };
            let opened = (|| {
                let file = files::validate_document(&file)?;
                self.events.focus()?;
                if self.has_unsaved()?
                    && self.dialogs.message(
                        "Unsaved text / 저장하지 않은 텍스트",
                        &["Keep editing / 계속 편집", "Open document / 문서 불러오기"],
                        0,
                    )? != 1
                {
                    return Ok(None);
                }
                self.load(&file).map(Some)
            })();
            match opened {
                Ok(Some(result)) => {
                    self.events
                        .emit("document:handoff", json!({"result":result,"error":null}))?;
                    self.events.focus()?;
                    return Ok(());
                }
                Ok(None) => {
                    lock(&self.state)?.handoff_active = false;
                }
                Err(error) => {
                    self.events.focus()?;
                    self.events.emit(
                        "document:handoff",
                        json!({"result":null,"error":error.message}),
                    )?;
                    return Ok(());
                }
            }
        }
    }
    fn load(&self, file: &Path) -> Result<Value> {
        self.previews.cancel();
        // Check the leaf before canonicalization; canonicalization alone grants symlinks.
        let file = files::validate_document(file)?;
        let root = file
            .parent()
            .ok_or_else(|| Error::new("Unsupported document"))?
            .to_owned();
        let document = self
            .engine
            .call(&root, "inspect", json!({"path":basename(&file)?}))?;
        let hash = required_text(&document, "sha256", "Invalid engine response")?.to_owned();
        let format = required_text(&document, "format", "Invalid engine response")?.to_owned();
        let name = required_text(&document, "name", "Invalid engine response")?.to_owned();
        let recent_id = match self.recents.record(&file, &format) {
            Ok(id) => Some(id),
            Err(error) => {
                eprintln!("Recent files unavailable: {}", error.message);
                None
            }
        };
        let base_bytes = files::read_document_bytes(&file, &hash)?;
        let session = Session {
            file: file.clone(),
            root,
            name,
            format,
            sha256: hash,
            recent_id: recent_id.clone(),
            base_bytes,
        };
        let draft = files::read_bounded(
            &files::draft_path(&self.profile, &file, &session.sha256),
            MAX_PATCH_BYTES,
        )
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null);
        let save_reviewed = {
            let mut state = lock(&self.state)?;
            self.previews.cancel();
            state.current = Some(session.clone());
            state.staged = Value::Null;
            state.selection = Value::Null;
            state.editing = false;
            state.output = None;
            state.previous = None;
            state.reviewed.contains(&file)
        };
        self.inbox.set_document(session);
        Ok(
            json!({"document":document,"draft":draft,"recentId":recent_id,"saveReviewed":save_reviewed,"agentLink":self.inbox.state()}),
        )
    }
    fn patch_session(&self, patch: &Value) -> Result<Session> {
        let current = self
            .current()
            .map_err(|_| Error::new("Invalid or stale patch"))?;
        validate_patch(patch, Some(&current.sha256))?;
        Ok(current)
    }
    pub fn prepare_preview(&self, args: &Value) -> Result<PreviewRequest> {
        check_arguments("document_preview", args)?;
        let patch = argument(args, "patch");
        let (current, token) = {
            let state = lock(&self.state)?;
            let current = state
                .current
                .clone()
                .ok_or_else(|| Error::new("Invalid or stale patch"))?;
            validate_patch(&patch, Some(&current.sha256))?;
            (current, self.previews.start())
        };
        Ok(PreviewRequest {
            current,
            patch,
            token,
        })
    }
    pub fn run_preview(&self, request: PreviewRequest) -> Result<Value> {
        let PreviewRequest {
            current,
            patch,
            token,
        } = request;
        let result = self.engine.call_cancelled(
            &current.root,
            "preview",
            json!({"path":basename(&current.file)?,"patch":patch}),
            token.cancel.clone(),
        );
        let is_current = self.previews.is_current(&token);
        self.previews.finish(&token);
        if !is_current {
            Ok(Value::Null)
        } else {
            result
        }
    }
    pub fn invoke(&self, command: &str, args: Value) -> Result<Value> {
        check_arguments(command, &args)?;
        if command == "document_preview" {
            return self.run_preview(self.prepare_preview(&args)?);
        }
        let _ordered = lock(&self.queue)?;
        self.invoke_ordered(command, args)
    }
    fn invoke_ordered(&self, command: &str, args: Value) -> Result<Value> {
        match command {
            "document_open" => match self.dialogs.open_file()? {
                Some(file) => self.load(&file),
                None => Ok(Value::Null),
            },
            "document_demo" => {
                self.load(&self.source_root.join("fixtures/generated/review-form.hwpx"))
            }
            "document_open_dropped" => {
                let file = PathBuf::from(required_text(&args, "path", "Unsupported document")?);
                if !lock(&self.state)?.drops.remove(&file) {
                    return Err(Error::new("Unsupported document"));
                }
                self.load(&file)
            }
            "document_bytes" => {
                let current = self.current()?;
                Ok(json!(files::read_document_bytes(
                    &current.file,
                    &current.sha256
                )?))
            }
            "document_recent" => self.recents.list(),
            "document_open_recent" => self.load(&self.recents.file_for(required_text(
                &args,
                "id",
                "Invalid recent document",
            )?)?),
            "document_cache_thumbnail" => {
                let current = self
                    .current()
                    .map_err(|_| Error::new("Invalid thumbnail document"))?;
                let id = required_text(&args, "id", "Invalid thumbnail document")?;
                if current.recent_id.as_deref() != Some(id) {
                    return Err(Error::new("Invalid thumbnail document"));
                }
                self.recents
                    .cache_thumbnail(id, required_text(&args, "png", "Invalid thumbnail")?)
                    .map(Value::Bool)
            }
            "document_format" => {
                let current = self
                    .current()
                    .map_err(|_| Error::new("Invalid format selection"))?;
                let node_id = argument(&args, "nodeId");
                if !node_id.is_null() && !node_id.is_string() {
                    return Err(Error::new("Invalid format selection"));
                }
                self.engine.call(
                    &current.root,
                    "format",
                    json!({"path":basename(&current.file)?,"nodeId":node_id}),
                )
            }
            "document_plan" => {
                let patch = argument(&args, "patch");
                let current = self.patch_session(&patch)?;
                self.engine.call(
                    &current.root,
                    "plan",
                    json!({"path":basename(&current.file)?,"patch":patch}),
                )
            }
            "document_save" => {
                self.save_copy(argument(&args, "patch"), argument(&args, "acknowledgement"))
            }
            "document_save_in_place" => {
                self.save_in_place(argument(&args, "patch"), argument(&args, "acknowledgement"))
            }
            "document_export_patch" => {
                let patch = argument(&args, "patch");
                let current = self.patch_session(&patch)?;
                let default = PathBuf::from(format!("{}.patch.json", current.file.display()));
                let Some(file) = self.dialogs.save_file(&default, &["json".into()])? else {
                    return Ok(Value::Null);
                };
                files::write_new(&file, &serde_json::to_vec_pretty(&patch)?)?;
                Ok(Value::Bool(true))
            }
            "document_draft" => {
                let patch = argument(&args, "patch");
                let current = self.patch_session(&patch)?;
                let selection = {
                    let mut state = lock(&self.state)?;
                    state.staged = patch.clone();
                    state.selection.clone()
                };
                self.inbox.update(selection, patch.clone());
                let file = files::draft_path(&self.profile, &current.file, &current.sha256);
                let parent = file
                    .parent()
                    .ok_or_else(|| Error::new("Invalid draft path"))?;
                files::ensure_private_directory(parent)?;
                files::atomic_write_json(&file, &patch)?;
                Ok(Value::Bool(true))
            }
            "shell_reveal_previous" | "shell_reveal_output" => {
                let file = {
                    let state = lock(&self.state)?;
                    if command == "shell_reveal_previous" {
                        state.previous.clone()
                    } else {
                        state.output.clone()
                    }
                };
                self.shell
                    .reveal(&file.ok_or_else(|| Error::new("No saved copy"))?)?;
                Ok(Value::Bool(true))
            }
            "document_editing" => {
                lock(&self.state)?.editing = boolean(&args, "value", "Invalid edit state")?;
                Ok(Value::Bool(true))
            }
            "document_handoff_ready" => {
                {
                    let mut state = lock(&self.state)?;
                    state.ready = true;
                    state.handoff_active = false;
                }
                self.flush_opens()?;
                Ok(Value::Bool(true))
            }
            "agent_link" => self.inbox.toggle(argument(&args, "enabled")),
            "agent_selection" => {
                let selection = argument(&args, "selection");
                if self.current().is_err()
                    || (!selection.is_null()
                        && (selection.get("nodeId").and_then(Value::as_str).is_none()
                            || selection
                                .get("text")
                                .and_then(Value::as_str)
                                .is_none_or(|text| text.encode_utf16().count() > 200000)))
                {
                    return Err(Error::new("Invalid selection"));
                }
                let staged = {
                    let mut state = lock(&self.state)?;
                    state.selection = selection.clone();
                    state.staged.clone()
                };
                self.inbox.update(selection, staged);
                Ok(Value::Bool(true))
            }
            "agent_decide" => self.inbox.decide(
                argument(&args, "file"),
                argument(&args, "ids"),
                argument(&args, "accepted"),
            ),
            "keyboard_scheme" => {
                let scheme = required_text(&args, "scheme", "Invalid keyboard scheme")?;
                if !["windows", "mac"].contains(&scheme) {
                    return Err(Error::new("Invalid keyboard scheme"));
                }
                self.events.key_scheme(scheme)?;
                files::atomic_write_json(
                    &self.profile.join("keyboard.json"),
                    &json!({"scheme":scheme}),
                )?;
                Ok(Value::Bool(true))
            }
            "clipboard_copy" => {
                let text = required_text(&args, "text", "Invalid clipboard text")?;
                if text.encode_utf16().count() > 4096 {
                    return Err(Error::new("Invalid clipboard text"));
                }
                self.shell.copy(text)?;
                Ok(Value::Bool(true))
            }
            "agents_status" => tools::status(
                self.engine.as_ref(),
                self.current()
                    .ok()
                    .as_ref()
                    .map(|session| session.root.as_path()),
            ),
            "agents_install_skill" => tools::install(
                self.engine.as_ref(),
                self.current()
                    .ok()
                    .as_ref()
                    .map(|session| session.root.as_path()),
                required_text(&args, "target", "Invalid skill target")?,
            ),
            "agents_copy_mcp" => {
                let client = required_text(&args, "client", "Invalid MCP client")?;
                if !["claude-code", "codex"].contains(&client) {
                    return Err(Error::new("Invalid MCP client"));
                }
                let text =
                    tools::copy_mcp(self.engine.as_ref(), Some(&self.current()?.root), client)?;
                self.shell.copy(&text)?;
                Ok(Value::String(text))
            }
            "agents_reveal_bundle" => {
                self.shell
                    .reveal(&tools::reveal_bundle(self.engine.as_ref())?)?;
                Ok(Value::Bool(true))
            }
            _ => Err(Error::new("Unknown desktop command")),
        }
    }
    fn save_copy(&self, patch: Value, acknowledgement: Value) -> Result<Value> {
        let current = self.patch_session(&patch)?;
        if acknowledgement != Value::Bool(true) {
            return Err(Error::new("Compatibility acknowledgement required"));
        }
        let extension = current
            .file
            .extension()
            .and_then(|value| value.to_str())
            .ok_or_else(|| Error::new("Unsupported document"))?;
        let stem = current
            .file
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| Error::new("Unsupported document"))?;
        let default = current.root.join(format!("{stem}-edited.{extension}"));
        let Some(output) = self.dialogs.save_file(&default, &[extension.into()])? else {
            return Ok(Value::Null);
        };
        let root = output
            .parent()
            .ok_or_else(|| Error::new("Unsupported document"))?
            .canonicalize()?;
        let temporary = tempfile::Builder::new()
            .prefix(".hwp-workbench-copy-")
            .tempdir_in(&root)?;
        let snapshot = temporary.path().join(basename(&current.file)?);
        files::write_new(&snapshot, &current.base_bytes)?;
        let relative = snapshot
            .strip_prefix(&root)
            .map_err(|_| Error::new("Unsafe snapshot path"))?;
        let mut report = self.engine.call(&root,"apply",json!({"path":relative,"output":basename(&output)?,"patch":patch,"acknowledgeRisks":true}))?;
        // The private snapshot is removed only after the engine returned its verification report.
        temporary.close()?;
        lock(&self.state)?.output = Some(root.join(basename(&output)?));
        if let Some(object) = report.as_object_mut() {
            object.insert("name".into(), Value::String(basename(&output)?));
        } else {
            return Err(Error::new("Invalid engine response"));
        }
        Ok(report)
    }
    fn save_in_place(&self, patch: Value, acknowledgement: Value) -> Result<Value> {
        let current = self.patch_session(&patch)?;
        {
            let mut state = lock(&self.state)?;
            if !state.reviewed.contains(&current.file) && acknowledgement != Value::Bool(true) {
                return Err(Error::new("Compatibility acknowledgement required"));
            }
            state.reviewed.insert(current.file.clone());
        }
        self.previews.cancel();
        let old_draft = files::draft_path(&self.profile, &current.file, &current.sha256);
        let result = self.engine.call(
            &current.root,
            "save",
            json!({"path":basename(&current.file)?,"patch":patch,"acknowledgeRisks":true}),
        );
        let report = match result {
            Ok(report) => report,
            Err(error) if error.code.as_deref() == Some("SOURCE_CHANGED") => {
                let answer = self.dialogs.message(
                    "Source changed / 원본 변경됨",
                    &[
                        "Reload / 불러오기 다시",
                        "Save As / 다른 이름으로 저장",
                        "Cancel / 취소",
                    ],
                    2,
                )?;
                return if answer == 0 {
                    Ok(json!({"action":"reload","opened":self.load(&current.file)?}))
                } else {
                    Ok(json!({"action":if answer == 1 {"saveAs"} else {"cancel"}}))
                };
            }
            Err(error) if error.code.as_deref() == Some("READ_ONLY_DOCUMENT") => {
                let answer = self.dialogs.message(
                    "Read-only document / 읽기 전용 문서",
                    &["Save As / 다른 이름으로 저장", "Cancel / 취소"],
                    0,
                )?;
                return Ok(json!({"action":if answer == 0 {"saveAs"} else {"cancel"}}));
            }
            Err(error) => return Err(error),
        };
        match std::fs::remove_file(old_draft) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        let opened = self.load(&current.file)?;
        let backup = Path::new(required_text(&report, "backup", "Invalid engine response")?);
        // Only a workspace-relative engine backup grants Finder reveal.
        if backup.is_absolute()
            || backup
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(Error::new("Invalid engine response"));
        }
        lock(&self.state)?.previous = Some(current.root.join(backup));
        Ok(
            json!({"action":"saved","opened":opened,"warnings":report.get("warnings").cloned().unwrap_or(Value::Null)}),
        )
    }
    pub fn key_scheme(&self) -> String {
        files::read_bounded(&self.profile.join("keyboard.json"), 1024)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| {
                value
                    .get("scheme")
                    .and_then(Value::as_str)
                    .filter(|scheme| ["windows", "mac"].contains(scheme))
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "windows".into())
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.stop();
    }
}

pub const COMMANDS: &[(&str, &[&str])] = &[
    ("document_open", &[]),
    ("document_open_dropped", &["path"]),
    ("document_demo", &[]),
    ("document_bytes", &[]),
    ("document_recent", &[]),
    ("document_open_recent", &["id"]),
    ("document_cache_thumbnail", &["id", "png"]),
    ("document_preview", &["patch"]),
    ("document_format", &["nodeId"]),
    ("document_plan", &["patch"]),
    ("document_save", &["patch", "acknowledgement"]),
    ("document_save_in_place", &["patch", "acknowledgement"]),
    ("document_export_patch", &["patch"]),
    ("document_draft", &["patch"]),
    ("shell_reveal_previous", &[]),
    ("shell_reveal_output", &[]),
    ("agents_status", &[]),
    ("agents_install_skill", &["target"]),
    ("agents_copy_mcp", &["client"]),
    ("agents_reveal_bundle", &[]),
    ("document_handoff_ready", &[]),
    ("document_editing", &["value"]),
    ("agent_link", &["enabled"]),
    ("agent_selection", &["selection"]),
    ("agent_decide", &["file", "ids", "accepted"]),
    ("keyboard_scheme", &["scheme"]),
    ("clipboard_copy", &["text"]),
];
pub(crate) fn check_arguments(command: &str, args: &Value) -> Result<()> {
    let (_, allowed) = COMMANDS
        .iter()
        .find(|(name, _)| *name == command)
        .ok_or_else(|| Error::new("Unknown desktop command"))?;
    let object = args
        .as_object()
        .ok_or_else(|| Error::new("Invalid desktop arguments"))?;
    if object.keys().any(|name| !allowed.contains(&name.as_str())) {
        let message = match command {
            "document_bytes" => "No document",
            "shell_reveal_previous" | "shell_reveal_output" => "Reveal accepts no arguments",
            "agents_status" => "Status accepts no arguments",
            "agents_reveal_bundle" => "Bundle reveal accepts no arguments",
            "document_handoff_ready" => "Handoff readiness accepts no arguments",
            _ => "Invalid desktop arguments",
        };
        return Err(Error::new(message));
    }
    Ok(())
}

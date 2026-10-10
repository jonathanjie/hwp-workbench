//! Agent-link inbox and current-document session publication.
//!
//! Watch events are hints. The engine owns bounded, no-follow proposal reads.
use crate::{engine::Engine, Error, Events, Session};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;
use time::OffsetDateTime;

// Match the desktop settings bound, decision limit, debounce and poll interval.
const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;
const MAX_DECISION_IDS: usize = 1000;
const SESSION_DEBOUNCE: Duration = Duration::from_millis(300);
const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Default)]
struct State {
    document: Option<Session>,
    enabled: bool,
    rows: Vec<Value>,
    decisions: HashMap<String, Map<String, Value>>,
    validated: HashMap<String, String>,
    settings: Map<String, Value>,
    selection: Value,
    staged: Value,
    error: Option<String>,
    generation: u64,
    session_revision: u64,
    scanning: bool,
    rescan: bool,
    watcher: Option<RecommendedWatcher>,
}

struct Inner {
    engine: Arc<dyn Engine>,
    events: Arc<dyn Events>,
    preferences: PathBuf,
    state: Mutex<State>,
    scan_finished: Condvar,
    preferences_write: Mutex<()>,
    decision_busy: Mutex<bool>,
    decision_available: Condvar,
}

struct DecisionPermit<'a> {
    inner: &'a Inner,
}

impl<'a> DecisionPermit<'a> {
    fn acquire(inner: &'a Inner) -> Self {
        let mut busy = lock(&inner.decision_busy);
        while *busy {
            busy = inner
                .decision_available
                .wait(busy)
                .unwrap_or_else(|error| error.into_inner());
        }
        *busy = true;
        Self { inner }
    }
}

impl Drop for DecisionPermit<'_> {
    fn drop(&mut self) {
        *lock(&self.inner.decision_busy) = false;
        self.inner.decision_available.notify_one();
    }
}

#[derive(Clone)]
pub struct AgentInbox {
    inner: Arc<Inner>,
}

// No user or engine code runs while state is locked. A failed worker must not
// make subsequent stop/state calls panic while the shell is shutting down.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

fn state_value(state: &State) -> Value {
    json!({
        "enabled": state.enabled,
        "proposals": if state.enabled { state.rows.clone() } else { Vec::new() },
        "error": state.error,
    })
}

fn record(state: &State, document: &Session) -> Value {
    // Milliseconds and UTC Z match Date.toISOString() in the desktop contract.
    let now = OffsetDateTime::now_utc();
    let updated_at = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.millisecond()
    );
    json!({
        "schemaVersion": 1, "app": "hwp-workbench", "appVersion": env!("CARGO_PKG_VERSION"),
        "document": {"name": document.name, "format": document.format, "sha256": document.sha256},
        "selection": state.selection, "staged": state.staged, "updatedAt": updated_at,
    })
}

fn is_current(inner: &Inner, generation: u64) -> bool {
    let state = lock(&inner.state);
    state.generation == generation && state.enabled
}

fn emit_current(inner: &Inner, generation: u64) {
    let payload = {
        let state = lock(&inner.state);
        if state.generation != generation {
            return;
        }
        state_value(&state)
    };
    let _ = inner.events.emit("agent:inbox", payload);
}

fn report_error(inner: &Inner, generation: u64, error: Error) {
    {
        let mut state = lock(&inner.state);
        if state.generation != generation {
            return;
        }
        state.error = Some(error.message);
    }
    emit_current(inner, generation);
}

fn stop_locked(state: &mut State) -> Option<RecommendedWatcher> {
    state.generation = state.generation.wrapping_add(1);
    state.session_revision = state.session_revision.wrapping_add(1);
    state.watcher.take()
}

impl AgentInbox {
    pub fn new(engine: Arc<dyn Engine>, events: Arc<dyn Events>, preferences: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                engine,
                events,
                preferences,
                state: Mutex::new(State::default()),
                scan_finished: Condvar::new(),
                preferences_write: Mutex::new(()),
                decision_busy: Mutex::new(false),
                decision_available: Condvar::new(),
            }),
        }
    }

    pub fn stop(&self) {
        let watcher = stop_locked(&mut lock(&self.inner.state));
        drop(watcher);
    }

    pub fn set_document(&self, document: Session) {
        let (generation, enabled) = {
            // Serialize settings IO so a previous toggle cannot replace settings
            // after a new document has already loaded them.
            let _preferences = lock(&self.inner.preferences_write);
            self.stop();
            let settings = read_settings(&self.inner.preferences).unwrap_or_default();
            let enabled =
                settings.get(document.file.to_string_lossy().as_ref()) == Some(&Value::Bool(true));
            let mut state = lock(&self.inner.state);
            state.document = Some(document);
            state.rows.clear();
            state.decisions.clear();
            state.validated.clear();
            state.selection = Value::Null;
            state.staged = Value::Null;
            state.settings = settings;
            state.enabled = enabled;
            (state.generation, enabled)
        };
        if enabled {
            self.start(generation);
        }
        emit_current(&self.inner, generation);
    }

    pub fn state(&self) -> Value {
        state_value(&lock(&self.inner.state))
    }

    pub fn toggle(&self, enabled: Value) -> Result<Value, Error> {
        let enabled = enabled
            .as_bool()
            .ok_or_else(|| Error::new("Invalid Agent link switch"))?;
        let generation = {
            let _preferences = lock(&self.inner.preferences_write);
            let (generation, settings, watcher) = {
                let mut state = lock(&self.inner.state);
                let file = state
                    .document
                    .as_ref()
                    .ok_or_else(|| Error::new("Invalid Agent link switch"))?
                    .file
                    .to_string_lossy()
                    .into_owned();
                let watcher = stop_locked(&mut state);
                state.enabled = enabled;
                state.settings.insert(file, Value::Bool(enabled));
                (state.generation, state.settings.clone(), watcher)
            };
            drop(watcher);
            write_settings(&self.inner.preferences, &settings)?;
            generation
        };
        if enabled {
            self.start(generation);
        }
        emit_current(&self.inner, generation);
        Ok(self.state())
    }

    pub fn update(&self, selection: Value, staged: Value) {
        let snapshot = {
            let mut state = lock(&self.inner.state);
            state.selection = selection;
            state.staged = if staged
                .get("operations")
                .and_then(Value::as_array)
                .is_some_and(|operations| !operations.is_empty())
            {
                staged
            } else {
                Value::Null
            };
            state.session_revision = state.session_revision.wrapping_add(1);
            state
                .enabled
                .then_some((state.generation, state.session_revision))
        };
        let Some((generation, revision)) = snapshot else {
            return;
        };
        let weak = Arc::downgrade(&self.inner);
        thread::spawn(move || {
            thread::sleep(SESSION_DEBOUNCE);
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let session = {
                let state = lock(&inner.state);
                if state.generation != generation
                    || state.session_revision != revision
                    || !state.enabled
                {
                    return;
                }
                state
                    .document
                    .as_ref()
                    .map(|document| (document.clone(), record(&state, document)))
            };
            if let Some((document, record)) = session {
                if let Err(error) =
                    inner
                        .engine
                        .agent_link(&document.root, &document.name, "session", record)
                {
                    report_error(&inner, generation, error);
                }
            }
        });
    }

    fn start(&self, generation: u64) {
        let snapshot = {
            let state = lock(&self.inner.state);
            if state.generation != generation || !state.enabled {
                return;
            }
            state
                .document
                .as_ref()
                .map(|document| (document.clone(), record(&state, document)))
        };
        let Some((document, record)) = snapshot else {
            return;
        };
        match self
            .inner
            .engine
            .agent_link(&document.root, &document.name, "session", record)
        {
            Ok(_) if is_current(&self.inner, generation) => {
                self.scan();
                if is_current(&self.inner, generation) {
                    self.watch(generation, &document.root.join(".hwp-workbench/proposals"));
                }
            }
            Ok(_) => return,
            Err(error) => report_error(&self.inner, generation, error),
        }
        if !is_current(&self.inner, generation) {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        thread::spawn(move || loop {
            thread::sleep(POLL_INTERVAL);
            let Some(inner) = weak.upgrade() else {
                return;
            };
            if !is_current(&inner, generation) {
                return;
            }
            request_scan(&inner, false);
        });
    }

    fn watch(&self, generation: u64, folder: &Path) {
        let Ok(info) = fs::symlink_metadata(folder) else {
            return;
        };
        if !info.is_dir() || info.file_type().is_symlink() {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            watch_event(&inner, generation, event);
        });
        let Ok(mut watcher) = watcher else {
            return;
        };
        if watcher.watch(folder, RecursiveMode::NonRecursive).is_err() {
            return;
        }
        let mut state = lock(&self.inner.state);
        if state.generation == generation && state.enabled {
            state.watcher = Some(watcher);
        }
    }

    /// A caller during a running scan waits for that scan and one more pass.
    pub fn scan(&self) {
        request_scan(&self.inner, true);
    }

    pub fn decide(&self, file: Value, ids: Value, accepted: Value) -> Result<Value, Error> {
        let ids = ids
            .as_array()
            .filter(|ids| !ids.is_empty() && ids.len() <= MAX_DECISION_IDS)
            .ok_or_else(|| Error::new("Invalid proposal decision"))?;
        let accepted = accepted
            .as_bool()
            .ok_or_else(|| Error::new("Invalid proposal decision"))?;
        // Keep decisions atomic across callers. The permit holds no mutex
        // during engine calls or event emission, so document changes can still
        // invalidate an action that is waiting for the engine.
        let _decision = DecisionPermit::acquire(&self.inner);
        let (generation, document, displayed) = {
            let state = lock(&self.inner.state);
            if !state.enabled {
                return Err(Error::new("Invalid proposal decision"));
            }
            let document = state
                .document
                .clone()
                .ok_or_else(|| Error::new("Invalid proposal decision"))?;
            let displayed = state
                .rows
                .iter()
                .find(|row| row.get("file") == Some(&file))
                .and_then(|row| row.get("patch"))
                .cloned();
            (state.generation, document, displayed)
        };
        let rows = read_proposals(&self.inner, &document)?;
        ensure_current(&self.inner, generation)?;
        let row = rows
            .iter()
            .find(|row| row.get("file") == Some(&file))
            .ok_or_else(stale_proposal)?;
        let patch = row
            .get("patch")
            .filter(|patch| !patch.is_null())
            .ok_or_else(stale_proposal)?;
        if row.get("status").and_then(Value::as_str) != Some("pending")
            || displayed.as_ref().map(serde_json::to_string).transpose()?
                != Some(serde_json::to_string(patch)?)
        {
            return Err(stale_proposal());
        }
        plan(&self.inner, &document, patch)?;
        ensure_current(&self.inner, generation)?;
        let file_name = file.as_str().ok_or_else(stale_proposal)?;
        let operations = patch
            .get("operations")
            .and_then(Value::as_array)
            .ok_or_else(stale_proposal)?;
        let mut decisions = {
            let state = lock(&self.inner.state);
            ensure_reviewed_patch(&state, generation, &file, patch)?;
            state.decisions.get(file_name).cloned().unwrap_or_default()
        };
        let mut targets = HashSet::new();
        for id in ids {
            let Some(id) = id.as_str() else {
                return Err(Error::new("Invalid or repeated proposal decision"));
            };
            if !targets.insert(id)
                || decisions.contains_key(id)
                || !operations
                    .iter()
                    .any(|operation| operation.get("nodeId").and_then(Value::as_str) == Some(id))
            {
                return Err(Error::new("Invalid or repeated proposal decision"));
            }
            decisions.insert(
                id.into(),
                json!(if accepted { "accepted" } else { "declined" }),
            );
        }
        let accepted_ids: Vec<_> = decisions
            .iter()
            .filter(|(_, status)| status.as_str() == Some("accepted"))
            .map(|(id, _)| id.clone())
            .collect();
        let declined_ids: Vec<_> = decisions
            .iter()
            .filter(|(_, status)| status.as_str() == Some("declined"))
            .map(|(id, _)| id.clone())
            .collect();
        let complete = decisions.len() == operations.len();
        if complete {
            {
                let state = lock(&self.inner.state);
                ensure_reviewed_patch(&state, generation, &file, patch)?;
            }
            let outcome = json!({
                "file": file,
                "acceptedNodeIds": accepted_ids,
                "declinedNodeIds": declined_ids,
                "expectedPatch": patch,
            });
            self.inner
                .engine
                .agent_link(&document.root, &document.name, "outcome", outcome)?;
        }
        {
            let mut state = lock(&self.inner.state);
            if state.generation != generation || !state.enabled {
                return Err(stale_proposal());
            }
            // A full outcome is already bound to the reviewed patch by the
            // engine. Watcher scans may have observed that completed outcome.
            if !complete {
                ensure_reviewed_patch(&state, generation, &file, patch)?;
            }
            state.decisions.insert(file_name.into(), decisions);
        }
        self.scan();
        ensure_current(&self.inner, generation)?;
        let selected_operations: Vec<Value> = if accepted {
            operations
                .iter()
                .filter(|operation| {
                    operation
                        .get("nodeId")
                        .and_then(Value::as_str)
                        .is_some_and(|id| targets.contains(id))
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        Ok(json!({
            "author": row.get("author").cloned().unwrap_or(Value::Null),
            "operations": selected_operations,
        }))
    }
}

fn stale_proposal() -> Error {
    Error::new("Stale proposal · reopen")
}

fn ensure_current(inner: &Inner, generation: u64) -> Result<(), Error> {
    if is_current(inner, generation) {
        Ok(())
    } else {
        Err(stale_proposal())
    }
}

fn ensure_reviewed_patch(
    state: &State,
    generation: u64,
    file: &Value,
    patch: &Value,
) -> Result<(), Error> {
    if state.generation != generation || !state.enabled {
        return Err(stale_proposal());
    }
    let displayed = state
        .rows
        .iter()
        .find(|row| row.get("file") == Some(file))
        .ok_or_else(stale_proposal)?;
    if displayed.get("status").and_then(Value::as_str) != Some("pending")
        || displayed
            .get("patch")
            .map(serde_json::to_string)
            .transpose()?
            != Some(serde_json::to_string(patch)?)
    {
        return Err(stale_proposal());
    }
    Ok(())
}

fn read_proposals(inner: &Inner, document: &Session) -> Result<Vec<Value>, Error> {
    let rows = inner.engine.agent_link(
        &document.root,
        &document.name,
        "read",
        json!({"sha256":document.sha256}),
    )?;
    rows.as_array()
        .cloned()
        .ok_or_else(|| Error::new("Invalid proposal rows"))
}

fn plan(inner: &Inner, document: &Session, patch: &Value) -> Result<(), Error> {
    inner.engine.call(
        &document.root,
        "plan",
        json!({"path":document.name,"patch":patch}),
    )?;
    Ok(())
}

fn watch_event(inner: &Arc<Inner>, generation: u64, event: notify::Result<notify::Event>) {
    if !is_current(inner, generation) {
        return;
    }
    if event.is_ok() {
        request_scan(inner, false);
    } else {
        // Drop the watcher outside its own callback thread.
        let inner = Arc::clone(inner);
        thread::spawn(move || {
            let watcher = {
                let mut state = lock(&inner.state);
                if state.generation != generation {
                    return;
                }
                state.watcher.take()
            };
            drop(watcher);
        });
    }
}

fn request_scan(inner: &Arc<Inner>, wait: bool) {
    let mut state = lock(&inner.state);
    if !state.enabled {
        return;
    }
    if state.scanning {
        state.rescan = true;
        if wait {
            while state.scanning {
                state = inner
                    .scan_finished
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
        }
        return;
    }
    state.scanning = true;
    drop(state);
    if wait {
        scan_until_current(inner);
    } else {
        let inner = Arc::clone(inner);
        thread::spawn(move || scan_until_current(&inner));
    }
}

fn scan_until_current(inner: &Arc<Inner>) {
    loop {
        let snapshot = {
            let mut state = lock(&inner.state);
            state.rescan = false;
            if state.enabled {
                state
                    .document
                    .clone()
                    .map(|document| (state.generation, document))
            } else {
                None
            }
        };
        if let Some((generation, document)) = snapshot {
            if let Err(error) = scan_once(inner, generation, &document) {
                report_error(inner, generation, error);
            }
        }
        let mut state = lock(&inner.state);
        if state.rescan && state.enabled {
            continue;
        }
        state.scanning = false;
        inner.scan_finished.notify_all();
        return;
    }
}

fn scan_once(inner: &Inner, generation: u64, document: &Session) -> Result<(), Error> {
    let mut rows = read_proposals(inner, document)?;
    for row in &mut rows {
        if !is_current(inner, generation) {
            return Ok(());
        }
        if row.get("status").and_then(Value::as_str) != Some("pending") {
            continue;
        }
        let file = row
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new("Invalid proposal rows"))?
            .to_owned();
        let patch = row
            .get("patch")
            .cloned()
            .ok_or_else(|| Error::new("Invalid proposal rows"))?;
        let fingerprint = hex::encode(Sha256::digest(serde_json::to_vec(&patch)?));
        let validate = {
            let mut state = lock(&inner.state);
            if state.generation != generation {
                return Ok(());
            }
            let validate = state.validated.get(&file) != Some(&fingerprint);
            if validate {
                state.decisions.remove(&file);
            }
            validate
        };
        if validate {
            match plan(inner, document, &patch) {
                Ok(()) => {
                    let mut state = lock(&inner.state);
                    if state.generation != generation {
                        return Ok(());
                    }
                    state.validated.insert(file.clone(), fingerprint);
                }
                Err(error) => {
                    let message = error.message.to_ascii_lowercase();
                    if let Some(row) = row.as_object_mut() {
                        row.insert(
                            "status".into(),
                            json!(
                                if message.contains("hash") || message.contains("source changed") {
                                    "stale"
                                } else {
                                    "unreadable"
                                }
                            ),
                        );
                        row.remove("patch");
                    }
                }
            }
        }
        let decisions = {
            let state = lock(&inner.state);
            if state.generation != generation {
                return Ok(());
            }
            state.decisions.get(&file).cloned().unwrap_or_default()
        };
        if let Some(row) = row.as_object_mut() {
            row.insert("decisions".into(), Value::Object(decisions));
        }
    }
    {
        let mut state = lock(&inner.state);
        if state.generation != generation {
            return Ok(());
        }
        state.rows = rows;
        state.error = None;
    }
    emit_current(inner, generation);
    Ok(())
}

fn read_settings(path: &Path) -> Result<Map<String, Value>, Error> {
    let info = fs::symlink_metadata(path)?;
    if !info.is_file() || info.file_type().is_symlink() || info.len() > MAX_SETTINGS_BYTES {
        return Ok(Map::new());
    }
    let descriptor = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    let file = File::from(descriptor);
    let info = file.metadata()?;
    if !info.is_file() || info.len() > MAX_SETTINGS_BYTES {
        return Ok(Map::new());
    }
    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Ok(Map::new());
    }
    let settings: Value = serde_json::from_slice(&bytes)?;
    Ok(settings.as_object().cloned().unwrap_or_default())
}

fn write_settings(path: &Path, settings: &Map<String, Value>) -> Result<(), Error> {
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|error| Error::new(error.to_string()))?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}", hex::encode(random)));
    let temporary = PathBuf::from(temporary);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Only a successfully created private file belongs to this operation.
    let mut output = options.open(&temporary)?;
    let result = (|| {
        output.write_all(&serde_json::to_vec(settings)?)?;
        match fs::symlink_metadata(path) {
            Ok(info) if !info.is_file() || info.file_type().is_symlink() => {
                return Err(Error::new("Unsafe Agent link settings"))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    drop(output);
    let cleanup = fs::remove_file(&temporary);
    if let Err(error) = cleanup {
        if error.kind() != std::io::ErrorKind::NotFound && result.is_ok() {
            return Err(error.into());
        }
    }
    result
}

#[cfg(test)]
#[path = "inbox/tests.rs"]
mod tests;

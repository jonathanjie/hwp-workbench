//! Private bounded proposal files and byte-bound desktop review outcomes.
use hwp_core::document::{patch_input, sha, Document};
use hwp_core::errors::{require, Result, WorkbenchError};
use hwp_core::workspace::Workspace;
use regex::Regex;
use rustix::fs::{self, AtFlags, FileType, Mode, OFlags};
use rustix::io::Errno;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

/// Bound each agent-link JSON file independently of native document size.
pub const MAX_LINK_FILE: usize = 2 * 1024 * 1024;
/// Bound the proposal inbox inventory in one session response.
pub const MAX_PROPOSALS: usize = 200;

#[cfg(test)]
thread_local! {static TEST_IDENTITY: std::cell::RefCell<Option<(String,String)>> = const {std::cell::RefCell::new(None)};}
#[cfg(test)]
pub(crate) fn test_identity(value: Option<(String, String)>) {
    TEST_IDENTITY.with(|slot| *slot.borrow_mut() = value);
}
fn identity() -> Result<(String, String)> {
    #[cfg(test)]
    if let Some(value) = TEST_IDENTITY.with(|slot| slot.borrow().clone()) {
        return Ok(value);
    }
    let now = time::OffsetDateTime::now_utc();
    let created = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let mut random = [0u8; 3];
    getrandom::fill(&mut random)
        .map_err(|error| WorkbenchError::new("WRITE_FAILED", error.to_string()))?;
    Ok((created, hex::encode(random)))
}
fn link_folder(workspace: &Workspace, path: &str) -> Result<String> {
    let relative = workspace.relative_path(path)?;
    let parent = Path::new(&relative)
        .parent()
        .and_then(Path::to_str)
        .unwrap_or("");
    Ok(if parent.is_empty() {
        ".hwp-workbench".into()
    } else {
        format!("{parent}/.hwp-workbench")
    })
}
pub fn propose(
    workspace: &Workspace,
    path: &str,
    document: &mut Document,
    args: &Value,
) -> Result<Value> {
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| {
            !s.trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}'))
                .is_empty()
        })
        .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "Supply a nonempty proposal title."))?;
    let author = args.get("author").unwrap_or(&Value::Null);
    let author = if args.get("author").is_none() {
        "agent"
    } else {
        author
            .as_str()
            .filter(|s| {
                !s.trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}'))
                    .is_empty()
            })
            .ok_or_else(|| {
                WorkbenchError::new("BAD_ARGUMENTS", "Supply a nonempty proposal author.")
            })?
    };
    require(
        args.get("notes").is_none_or(Value::is_string),
        "BAD_ARGUMENTS",
        "Proposal notes must be text.",
    )?;
    let patch = patch_input(args.get("patch").unwrap_or(&Value::Null));
    document.patch(patch)?;
    let mut native_patch = patch.clone();
    if let Some(operations) = native_patch
        .get_mut("operations")
        .and_then(Value::as_array_mut)
    {
        for operation in operations {
            if let Some(id) = operation.get("nodeId").and_then(Value::as_str) {
                let id = document.resolve_node_id(id)?;
                if let Some(fields) = operation.as_object_mut() {
                    fields.insert("nodeId".into(), json!(id));
                }
            }
        }
    }
    let (created, random) = identity()?;
    let mut proposal = json!({"schemaVersion":1,"kind":"hwp-workbench-proposal","document":{"name":document.name,"sha256":document.hash},"title":title,"author":author,"createdAt":created,"patch":native_patch});
    if let Some(notes) = args.get("notes") {
        if let Some(fields) = proposal.as_object_mut() {
            fields.insert("notes".into(), notes.clone());
        }
    }
    let mut content = serde_json::to_vec_pretty(&proposal)
        .map_err(|error| WorkbenchError::new("REQUEST_FAILED", error.to_string()))?;
    content.push(b'\n');
    require(
        content.len() <= MAX_LINK_FILE,
        "LIMIT",
        "Proposal exceeds 2 MiB.",
    )?;
    require(
        sha(&workspace.read(path)?) == document.hash,
        "STALE_DOCUMENT",
        "Source changed during processing; proposal was not published.",
    )?;
    let folder = format!("{}/proposals", link_folder(workspace, path)?);
    workspace.ensure_directory(&folder)?;
    let timestamp = created.replace(['-', ':'], "");
    let destination = format!("{folder}/{}.{timestamp}-{random}.json", document.name);
    workspace.write_new(&destination, &content)?;
    Ok(json!({"proposal":destination,"changes":document.diff().get("changes")}))
}
struct ReadJson {
    value: Option<Value>,
    readable: bool,
    missing: bool,
    digest: Option<String>,
}
fn read_json(workspace: &Workspace, path: &str) -> ReadJson {
    match workspace.read_with_options(path, MAX_LINK_FILE, true) {
        Ok(None) => ReadJson {
            value: None,
            readable: true,
            missing: true,
            digest: None,
        },
        Ok(Some(bytes)) => match serde_json::from_slice(&bytes) {
            Ok(value) => ReadJson {
                value: Some(value),
                readable: true,
                missing: false,
                digest: Some(sha(&bytes)),
            },
            Err(_) => ReadJson {
                value: None,
                readable: false,
                missing: false,
                digest: None,
            },
        },
        Err(_) => ReadJson {
            value: None,
            readable: false,
            missing: false,
            digest: None,
        },
    }
}
fn version(value: &Value) -> bool {
    value.is_object() && value.get("schemaVersion").and_then(Value::as_u64) == Some(1)
}
fn text(value: &Value, key: &str) -> bool {
    value.get(key).is_some_and(Value::is_string)
}
fn valid_patch(value: &Value) -> bool {
    if !version(value) || !text(value, "baseSha256") {
        return false;
    }
    value
        .get("operations")
        .and_then(Value::as_array)
        .is_some_and(|ops| {
            !ops.is_empty()
                && ops.len() <= 1000
                && ops.iter().all(|operation| {
                    operation.as_object().is_some_and(|map| {
                        map.len() == 4
                            && map.get("op").and_then(Value::as_str) == Some("set_text")
                            && ["nodeId", "expectedText", "text"]
                                .iter()
                                .all(|key| text(operation, key))
                    })
                })
        })
}
fn valid_session(value: &Value, name: &str, format: &str, hash: &Value) -> bool {
    if !version(value)
        || value.get("app").and_then(Value::as_str) != Some("hwp-workbench")
        || !text(value, "appVersion")
        || !text(value, "updatedAt")
    {
        return false;
    }
    let identity = json!({"name":name,"format":format,"sha256":hash});
    if value.get("document") != Some(&identity)
        || value.get("selection").is_none()
        || value.get("staged").is_none()
    {
        return false;
    }
    if let Some(selection) = value.get("selection") {
        if !selection.is_null()
            && (!selection.is_object() || !text(selection, "nodeId") || !text(selection, "text"))
        {
            return false;
        }
    }
    value
        .get("staged")
        .is_some_and(|staged| staged.is_null() || valid_patch(staged))
}
fn valid_proposal(value: &Value, name: &str) -> bool {
    if !version(value)
        || value.get("kind").and_then(Value::as_str) != Some("hwp-workbench-proposal")
    {
        return false;
    }
    let Some(identity) = value.get("document").filter(|v| v.is_object()) else {
        return false;
    };
    if identity.get("name").and_then(Value::as_str) != Some(name)
        || !text(identity, "sha256")
        || !["title", "author", "createdAt"]
            .iter()
            .all(|key| text(value, key))
    {
        return false;
    }
    value.get("patch").is_some_and(|patch| {
        valid_patch(patch) && patch.get("baseSha256") == identity.get("sha256")
    })
}
fn node_ids<'a>(value: &'a Value, key: &str) -> Option<Vec<&'a str>> {
    let array = value.get(key)?.as_array()?;
    if array.len() > 1000 {
        return None;
    }
    array.iter().map(Value::as_str).collect()
}
fn outcome_matches(outcome: &Value, proposal: &Value, digest: Option<&str>) -> bool {
    if !version(outcome)
        || !text(outcome, "decidedAt")
        || !matches!(
            outcome.get("status").and_then(Value::as_str),
            Some("accepted" | "declined" | "partial")
        )
    {
        return false;
    }
    if outcome.get("proposalSha256").and_then(Value::as_str) != digest {
        return false;
    }
    let Some(accepted) = node_ids(outcome, "acceptedNodeIds") else {
        return false;
    };
    let Some(declined) = node_ids(outcome, "declinedNodeIds") else {
        return false;
    };
    let proposed: Vec<_> = proposal
        .get("patch")
        .and_then(|p| p.get("operations"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|op| op.get("nodeId").and_then(Value::as_str))
        .collect();
    let proposed_set: HashSet<_> = proposed.iter().copied().collect();
    let combined_set: HashSet<_> = accepted.iter().chain(&declined).copied().collect();
    if proposed_set.len() != proposed.len()
        || combined_set.len() != accepted.len() + declined.len()
        || combined_set != proposed_set
    {
        return false;
    }
    let expected = if !accepted.is_empty() && !declined.is_empty() {
        "partial"
    } else if !accepted.is_empty() {
        "accepted"
    } else {
        "declined"
    };
    outcome.get("status").and_then(Value::as_str) == Some(expected)
}
fn proposal_summary(workspace: &Workspace, path: &str, name: &str, hash: &Value) -> Value {
    let mut summary = json!({"file":path,"title":null,"author":null,"createdAt":null,"operations":0,"status":"unreadable"});
    let read = read_json(workspace, path);
    let Some(proposal) = read
        .value
        .filter(|v| read.readable && valid_proposal(v, name))
    else {
        return summary;
    };
    for key in ["title", "author", "createdAt"] {
        if let Some(value) = proposal.get(key) {
            if let Some(fields) = summary.as_object_mut() {
                fields.insert(key.into(), value.clone());
            }
        }
    }
    let operations = proposal
        .get("patch")
        .and_then(|p| p.get("operations"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let outcome_path = Path::new(path).with_extension("outcome.json");
    let outcome = read_json(workspace, outcome_path.to_str().unwrap_or(""));
    let status = if !outcome.readable || !outcome.missing {
        if outcome.readable
            && outcome
                .value
                .as_ref()
                .is_some_and(|value| outcome_matches(value, &proposal, read.digest.as_deref()))
        {
            outcome
                .value
                .as_ref()
                .and_then(|v| v.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("needsReview")
        } else {
            "needsReview"
        }
    } else if proposal.get("document").and_then(|v| v.get("sha256")) == Some(hash) {
        "pending"
    } else {
        "stale"
    };
    if let Some(fields) = summary.as_object_mut() {
        fields.insert("operations".into(), json!(operations));
        fields.insert("status".into(), json!(status));
    }
    summary
}
pub fn session(workspace: &Workspace, path: &str, document: &Document) -> Result<Value> {
    let folder = link_folder(workspace, path)?;
    let record = read_json(
        workspace,
        &format!("{folder}/{}.session.json", document.name),
    );
    let linked = record.readable
        && record.value.as_ref().is_some_and(|value| {
            valid_session(
                value,
                &document.name,
                &document.format,
                &json!(document.hash),
            )
        });
    let proposal_folder = format!("{folder}/proposals");
    let pattern = Regex::new(&format!(
        r"^{}\.[0-9]{{8}}T[0-9]{{6}}Z-[a-f0-9]{{6}}\.json$",
        regex::escape(&document.name)
    ))
    .map_err(|error| WorkbenchError::new("REQUEST_FAILED", error.to_string()))?;
    let filenames = workspace.list_files(
        &proposal_folder,
        |name| pattern.is_match(name),
        MAX_PROPOSALS,
    )?;
    let proposals: Vec<_> = filenames
        .iter()
        .map(|name| {
            proposal_summary(
                workspace,
                &format!("{proposal_folder}/{name}"),
                &document.name,
                &json!(document.hash),
            )
        })
        .collect();
    Ok(json!({"linked":linked,"session":if linked{record.value}else{None},"proposals":proposals}))
}

/// Validate the basename before any desktop link-file access.
pub fn validate_document_name(name: &str) -> Result<()> {
    let path = Path::new(name);
    require(
        path.file_name().and_then(|part| part.to_str()) == Some(name)
            && path
                .extension()
                .and_then(|part| part.to_str())
                .is_some_and(|extension| {
                    matches!(extension.to_ascii_lowercase().as_str(), "hwp" | "hwpx")
                }),
        "BAD_ARGUMENTS",
        "Invalid document name.",
    )
}
fn proposal_pattern(name: &str) -> Result<Regex> {
    Regex::new(&format!(
        r"\A{}\.[0-9]{{8}}T[0-9]{{6}}Z-[a-f0-9]{{6}}\.json\z",
        regex::escape(name)
    ))
    .map_err(|error| WorkbenchError::new("REQUEST_FAILED", error.to_string()))
}
fn read_proposals(workspace: &Workspace, name: &str, hash: &Value) -> Result<Value> {
    let folder = ".hwp-workbench/proposals";
    let pattern = proposal_pattern(name)?;
    let filenames =
        workspace.list_files(folder, |filename| pattern.is_match(filename), MAX_PROPOSALS)?;
    let mut rows = Vec::new();
    for filename in filenames {
        let path = format!("{folder}/{filename}");
        let mut summary = proposal_summary(workspace, &path, name, hash);
        if let Some(fields) = summary.as_object_mut() {
            fields.insert("file".into(), json!(filename));
            if matches!(
                fields.get("status").and_then(Value::as_str),
                Some("pending" | "stale")
            ) {
                let read = read_json(workspace, &path);
                if let Some(proposal) = read
                    .value
                    .filter(|value| read.readable && valid_proposal(value, name))
                {
                    if let Some(patch) = proposal.get("patch") {
                        fields.insert("patch".into(), patch.clone());
                    }
                }
            }
        }
        rows.push(summary);
    }
    Ok(json!(rows))
}
fn write_error(error: impl std::fmt::Display) -> WorkbenchError {
    WorkbenchError::new("WRITE_FAILED", error.to_string())
}
fn session_temporary_name() -> Result<String> {
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).map_err(write_error)?;
    Ok(format!(".session-{}", hex::encode(bytes)))
}
fn publish_session(workspace: &Workspace, name: &str, record: &Value) -> Result<Value> {
    let format = Path::new(name)
        .extension()
        .and_then(|part| part.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let hash = record
        .get("document")
        .and_then(|value| value.get("sha256"))
        .unwrap_or(&Value::Null);
    require(
        valid_session(record, name, &format, hash),
        "BAD_ARGUMENTS",
        "Invalid session.",
    )?;
    workspace.ensure_directory(".hwp-workbench/proposals")?;
    let mut payload = crate::mcp::contract_json(record).into_bytes();
    payload.push(b'\n');
    require(
        payload.len() <= MAX_LINK_FILE,
        "LIMIT",
        "Session exceeds 2 MiB.",
    )?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let root = fs::open(workspace.root(), flags, Mode::empty()).map_err(write_error)?;
    let parent = fs::openat(&root, ".hwp-workbench", flags, Mode::empty()).map_err(write_error)?;
    let destination = format!("{name}.session.json");
    match fs::statat(&parent, destination.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(info) => require(
            FileType::from_raw_mode(info.st_mode) == FileType::RegularFile,
            "UNSAFE_PATH",
            "Session is nonregular or a symlink.",
        )?,
        Err(Errno::NOENT) => {}
        Err(error) => return Err(write_error(error)),
    }
    let temporary = session_temporary_name()?;
    publish_session_bytes(&parent, &destination, &temporary, &payload)?;
    Ok(json!(true))
}
fn publish_session_bytes(
    parent: &rustix::fd::OwnedFd,
    destination: &str,
    temporary: &str,
    payload: &[u8],
) -> Result<()> {
    // A failed exclusive create does not grant ownership of the temporary name.
    let descriptor = fs::openat(
        parent,
        temporary,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(write_error)?;
    let result = (|| {
        let mut output = std::fs::File::from(descriptor);
        output.write_all(payload).map_err(write_error)?;
        output.sync_all().map_err(write_error)?;
        fs::renameat(parent, temporary, parent, destination).map_err(write_error)
    })();
    let cleanup = fs::unlinkat(parent, temporary, AtFlags::empty());
    result?;
    match cleanup {
        Ok(()) | Err(Errno::NOENT) => Ok(()),
        Err(error) => Err(write_error(error)),
    }
}
fn publish_outcome(workspace: &Workspace, name: &str, args: &Value) -> Result<Value> {
    let pattern = proposal_pattern(name)?;
    let filename = args
        .get("file")
        .and_then(Value::as_str)
        .filter(|file| pattern.is_match(file))
        .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "Invalid proposal file."))?;
    let read = read_json(workspace, &format!(".hwp-workbench/proposals/{filename}"));
    let proposal = read
        .value
        .filter(|value| read.readable && valid_proposal(value, name))
        .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "Invalid proposal."))?;
    if let Some(expected) = args.get("expectedPatch") {
        require(
            proposal.get("patch") == Some(expected),
            "STALE_DOCUMENT",
            "Proposal changed before the outcome write.",
        )?;
    }
    // Desktop publication checks list types first, then the complete partition.
    // Apply the 1000-ID cap when validating stored outcomes.
    let outcome_ids = |key| -> Option<Vec<&str>> {
        args.get(key)?
            .as_array()?
            .iter()
            .map(Value::as_str)
            .collect()
    };
    let accepted = outcome_ids("acceptedNodeIds");
    let declined = outcome_ids("declinedNodeIds");
    let (Some(accepted), Some(declined)) = (accepted, declined) else {
        return Err(WorkbenchError::new(
            "BAD_ARGUMENTS",
            "Invalid outcome node IDs.",
        ));
    };
    let proposed: Vec<_> = proposal
        .get("patch")
        .and_then(|patch| patch.get("operations"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|operation| operation.get("nodeId").and_then(Value::as_str))
        .collect();
    let proposed_set: HashSet<_> = proposed.iter().copied().collect();
    let combined: HashSet<_> = accepted.iter().chain(&declined).copied().collect();
    require(
        proposed_set.len() == proposed.len()
            && combined.len() == accepted.len() + declined.len()
            && combined == proposed_set,
        "BAD_ARGUMENTS",
        "Outcome must contain the complete proposal once.",
    )?;
    let status = if !accepted.is_empty() && !declined.is_empty() {
        "partial"
    } else if !accepted.is_empty() {
        "accepted"
    } else {
        "declined"
    };
    let outcome = json!({"schemaVersion":1,"status":status,"proposalSha256":read.digest,
        "acceptedNodeIds":accepted,"declinedNodeIds":declined,"decidedAt":identity()?.0});
    let stem = filename
        .strip_suffix(".json")
        .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "Invalid proposal file."))?;
    let mut bytes = crate::mcp::contract_json(&outcome).into_bytes();
    bytes.push(b'\n');
    workspace.write_new(
        &format!(".hwp-workbench/proposals/{stem}.outcome.json"),
        &bytes,
    )?;
    Ok(outcome)
}
/// The desktop helper's fixed actions reuse the CLI's link validators and workspace.
pub fn desktop_action(
    workspace: &Workspace,
    name: &str,
    action: &str,
    args: &Value,
) -> Result<Value> {
    validate_document_name(name)?;
    match action {
        "read" => read_proposals(
            workspace,
            name,
            args.get("sha256")
                .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "'sha256'"))?,
        ),
        "session" => publish_session(workspace, name, args),
        "outcome" => publish_outcome(workspace, name, args),
        _ => Err(WorkbenchError::new("BAD_ARGUMENTS", "Unknown link action.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_temporary_collision_preserves_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let existing = directory.path().join(".session-collision");
        std::fs::write(&existing, b"another writer's bytes").unwrap();
        let parent = fs::open(
            directory.path(),
            OFlags::RDONLY | OFlags::DIRECTORY,
            Mode::empty(),
        )
        .unwrap();
        assert!(publish_session_bytes(
            &parent,
            "source.hwpx.session.json",
            ".session-collision",
            b"new session"
        )
        .is_err());
        assert_eq!(std::fs::read(existing).unwrap(), b"another writer's bytes");
        assert!(!directory.path().join("source.hwpx.session.json").exists());
    }
    #[test]
    fn collision_refused_without_overwrite() {
        let folder = tempfile::tempdir().unwrap();
        let bytes = include_bytes!("../../../fixtures/generated/review-form.hwpx");
        std::fs::write(folder.path().join("form.hwpx"), bytes).unwrap();
        let workspace = Workspace::new(folder.path()).unwrap();
        let mut document = Document::new(bytes, "form.hwpx").unwrap();
        let preview =
            crate::agent_ops::set_text(&document, &json!("s0:t0"), &json!("new")).unwrap();
        let args = json!({"patch":preview,"title":"Review"});
        test_identity(Some(("2026-10-09T10:20:30Z".into(), "abcdef".into())));
        let result = propose(&workspace, "form.hwpx", &mut document, &args).unwrap();
        let path = result["proposal"].as_str().unwrap();
        let before = workspace.read(path).unwrap();
        let mut document = Document::new(bytes, "form.hwpx").unwrap();
        assert_eq!(
            propose(&workspace, "form.hwpx", &mut document, &args)
                .unwrap_err()
                .code,
            "OUTPUT_EXISTS"
        );
        assert_eq!(workspace.read(path).unwrap(), before);
        test_identity(None);
    }
}

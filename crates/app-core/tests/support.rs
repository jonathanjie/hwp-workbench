use app_core::{files, preview::LatestPreview, recent::RecentStore};
use sha2::{Digest, Sha256};
use std::{fs, sync::atomic::Ordering};

#[test]
fn document_bytes_reject_stale_sources_oversized_files_and_replaced_symlink_grants() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = fs::canonicalize(scratch.path()).unwrap();
    let file = folder.join("source.hwpx");
    fs::write(&file, b"document").unwrap();
    let hash = hex::encode(Sha256::digest(b"document"));
    assert_eq!(
        files::read_document_bytes(&file, &hash).unwrap(),
        b"document"
    );
    fs::write(&file, b"changed").unwrap();
    assert!(files::read_document_bytes(&file, &hash)
        .unwrap_err()
        .message
        .contains("Source changed"));
    fs::remove_file(&file).unwrap();
    let other = folder.join("other.hwpx");
    fs::write(&other, b"document").unwrap();
    std::os::unix::fs::symlink(&other, &file).unwrap();
    assert!(files::read_document_bytes(&file, &hash)
        .unwrap_err()
        .message
        .contains("Source path changed"));
    fs::remove_file(&file).unwrap();
    fs::File::create(&file)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    assert!(files::read_document_bytes(&file, &hash)
        .unwrap_err()
        .message
        .contains("64 MiB"));
}

#[test]
fn drop_rejects_wrong_extensions_directories_and_symlinks() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = fs::canonicalize(scratch.path()).unwrap();
    let file = folder.join("doc.HWPX");
    fs::write(&file, b"sample").unwrap();
    assert_eq!(files::validate_document(&file).unwrap(), file);
    let text = folder.join("doc.txt");
    fs::write(&text, b"sample").unwrap();
    assert!(files::validate_document(&text)
        .unwrap_err()
        .message
        .contains("Unsupported"));
    let directory = folder.join("folder.hwp");
    fs::create_dir(&directory).unwrap();
    assert!(files::validate_document(&directory)
        .unwrap_err()
        .message
        .contains("regular"));
    let link = folder.join("link.hwp");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(files::validate_document(&link)
        .unwrap_err()
        .message
        .contains("regular"));
}

#[test]
fn recent_files_persist_eight_opaque_ids_and_reopen_only_known_regular_files() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = fs::canonicalize(scratch.path()).unwrap();
    let store = RecentStore::new(folder.join("data"));
    for number in 0..9 {
        let file = folder.join(format!("{number}.hwpx"));
        fs::write(&file, b"sample").unwrap();
        store.record(&file, "hwpx").unwrap();
    }
    let list = store.list().unwrap();
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 8);
    assert_eq!(rows[0]["name"], "8.hwpx");
    assert_eq!(rows[0].as_object().unwrap().len(), 5);
    let id = rows[0]["id"].as_str().unwrap();
    let reloaded = RecentStore::new(folder.join("data"));
    let file = folder.join("8.hwpx");
    assert_eq!(reloaded.file_for(id).unwrap(), file);
    assert_eq!(reloaded.record(&file, "hwpx").unwrap(), id);
    let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aWQAAAABJRU5ErkJggg==";
    assert!(reloaded.cache_thumbnail(id, png).unwrap());
    assert_eq!(reloaded.list().unwrap()[0]["thumbnail"], png);
    assert!(reloaded.cache_thumbnail("../outside", png).is_err());
    assert!(reloaded
        .cache_thumbnail(id, "data:image/svg+xml,evil")
        .is_err());
    assert!(reloaded
        .cache_thumbnail(id, &format!("data:image/png;base64,{}", "a".repeat(300000)))
        .is_err());
    fs::remove_file(file).unwrap();
    assert!(reloaded.file_for(id).is_err());
    assert!(reloaded.file_for("0").is_err());
}

#[test]
fn recent_store_and_drops_accept_symlinked_parent_folders_but_not_a_symlinked_leaf() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = fs::canonicalize(scratch.path()).unwrap();
    let real = folder.join("real");
    fs::create_dir(&real).unwrap();
    let linked = folder.join("linked");
    std::os::unix::fs::symlink(&real, &linked).unwrap();
    let file = real.join("doc.hwpx");
    fs::write(&file, b"sample").unwrap();
    let store = RecentStore::new(linked.join("profile"));
    let id = store.record(&file, "hwpx").unwrap();
    assert_eq!(store.list().unwrap()[0]["id"], id);
    assert_eq!(
        files::validate_document(&linked.join("doc.hwpx")).unwrap(),
        file
    );
    let store_link = folder.join("store-link");
    std::os::unix::fs::symlink(&real, &store_link).unwrap();
    assert!(RecentStore::new(store_link)
        .list()
        .unwrap_err()
        .message
        .contains("changed"));
}

#[test]
fn new_preview_aborts_old_work_and_drops_its_result() {
    let lane = LatestPreview::default();
    let first = lane.start();
    let second = lane.start();
    assert!(first.cancel.load(Ordering::SeqCst));
    assert!(!lane.is_current(&first));
    assert!(lane.is_current(&second));
    lane.finish(&first);
    assert!(lane.is_current(&second));
    lane.cancel();
    assert!(second.cancel.load(Ordering::SeqCst));
    assert!(!lane.is_current(&second));
}

#[test]
fn draft_keys_include_file_and_content() {
    let profile = std::path::Path::new("/profile");
    let first = files::draft_path(profile, std::path::Path::new("/a/form.hwpx"), "hash");
    let second = files::draft_path(profile, std::path::Path::new("/b/form.hwpx"), "hash");
    assert_ne!(first, second);
    assert_ne!(
        first,
        files::draft_path(profile, std::path::Path::new("/a/form.hwpx"), "newhash")
    );
    assert!(first
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with("-hash.json"));
}

#[test]
fn fixed_skill_actions_validate_targets_and_mcp_roots_come_from_main() {
    use app_core::{
        engine::{Engine, ToolAction},
        tools, Error,
    };
    use serde_json::{json, Value};
    use std::{
        path::{Path, PathBuf},
        sync::Mutex,
    };
    struct FakeEngine {
        actions: Mutex<Vec<ToolAction>>,
        data_home: PathBuf,
    }
    impl Engine for FakeEngine {
        fn bundle_path(&self) -> Result<PathBuf, Error> {
            Ok(hwp_workbench::mcpb::bundle_path_in(&self.data_home))
        }
        fn call(&self, _: &Path, _: &str, _: Value) -> Result<Value, Error> {
            unreachable!()
        }
        fn agent_link(&self, _: &Path, _: &str, _: &str, _: Value) -> Result<Value, Error> {
            unreachable!()
        }
        fn tool(&self, action: ToolAction) -> Result<String, Error> {
            let response = match &action {
                ToolAction::Status => {
                    json!({"claude":{"installed":false},"codex":{"installed":false}}).to_string()
                }
                ToolAction::McpConfig { root, .. } => {
                    format!("claude mcp add hwp-workbench {}\n", root.display())
                }
                _ => "true".to_owned(),
            };
            self.actions.lock().unwrap().push(action);
            Ok(response)
        }
    }
    let scratch = tempfile::tempdir().unwrap();
    let engine = FakeEngine {
        actions: Mutex::default(),
        data_home: scratch.path().join("app-data"),
    };
    let source = scratch.path();
    let status = tools::status(&engine, None).unwrap();
    assert_eq!(status["hasDocument"], false);
    assert_eq!(status["claudeDesktop"]["bundleAvailable"], false);
    assert!(tools::install(&engine, None, "other").is_err());
    tools::install(&engine, None, "codex").unwrap();
    assert!(engine
        .actions
        .lock()
        .unwrap()
        .contains(&ToolAction::Install {
            target: "codex".into()
        }));
    assert!(tools::copy_mcp(&engine, None, "codex")
        .unwrap_err()
        .message
        .contains("No document"));
    assert!(tools::copy_mcp(&engine, Some(source), "shell").is_err());
    assert!(tools::copy_mcp(&engine, Some(source), "claude-code")
        .unwrap()
        .contains(source.to_str().unwrap()));
    assert!(engine
        .actions
        .lock()
        .unwrap()
        .contains(&ToolAction::McpConfig {
            client: "claude-code".into(),
            root: PathBuf::from(source)
        }));
}

#[test]
fn write_new_preserves_existing_files_and_symlink_targets() {
    let scratch = tempfile::tempdir().unwrap();
    let file = scratch.path().join("patch.json");
    files::write_new(&file, b"original").unwrap();
    assert!(files::write_new(&file, b"replacement").is_err());
    assert_eq!(fs::read(&file).unwrap(), b"original");
    let link = scratch.path().join("link.json");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(files::write_new(&link, b"replacement").is_err());
    assert_eq!(fs::read(&file).unwrap(), b"original");
}

#[test]
fn replacing_a_document_parent_with_a_symlink_is_refused() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = fs::canonicalize(scratch.path()).unwrap();
    let original = folder.join("original");
    let other = folder.join("other");
    fs::create_dir(&original).unwrap();
    fs::create_dir(&other).unwrap();
    let file = original.join("source.hwpx");
    fs::write(&file, b"document").unwrap();
    fs::write(other.join("source.hwpx"), b"document").unwrap();
    fs::rename(&original, folder.join("moved")).unwrap();
    std::os::unix::fs::symlink(&other, &original).unwrap();
    let hash = hex::encode(Sha256::digest(b"document"));
    assert_eq!(
        files::read_document_bytes(&file, &hash)
            .unwrap_err()
            .message,
        "Source path changed"
    );
}

#[test]
fn concurrent_previews_cancel_every_prior_token() {
    use std::sync::{Arc, Barrier};
    let lane = Arc::new(LatestPreview::default());
    let barrier = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let lane = lane.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                lane.start()
            })
        })
        .collect();
    let tokens: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(
        tokens
            .iter()
            .filter(|token| !token.cancel.load(Ordering::SeqCst))
            .count(),
        1
    );
    assert_eq!(
        tokens.iter().filter(|token| lane.is_current(token)).count(),
        1
    );
}

#[test]
fn fifo_sources_and_profile_reads_are_refused_without_blocking() {
    use std::sync::mpsc;
    use std::time::Duration;
    let scratch = tempfile::tempdir().unwrap();
    let file = fs::canonicalize(scratch.path())
        .unwrap()
        .join("source.hwpx");
    #[cfg(not(target_os = "macos"))]
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &file,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    #[cfg(target_os = "macos")]
    assert!(std::process::Command::new("/usr/bin/mkfifo")
        .arg(&file)
        .status()
        .unwrap()
        .success());
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let source = files::read_document_bytes(&file, "hash");
        let profile = files::read_bounded(&file, 1024);
        sender.send((source, profile)).unwrap();
    });
    let (source, profile) = receiver
        .recv_timeout(Duration::from_millis(500))
        .expect("FIFO reads must not wait for a writer");
    assert!(source.is_err());
    assert!(profile.is_err());
}

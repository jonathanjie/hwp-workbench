use super::*;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use tempfile::TempDir;

struct Fixture {
    directory: TempDir,
    workspace: Workspace,
}
impl Fixture {
    fn new() -> Self {
        reset_hooks();
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("form.hwpx"),
            b"original document bytes",
        )
        .unwrap();
        fs::set_permissions(
            directory.path().join("form.hwpx"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        let workspace = Workspace::new(directory.path()).unwrap();
        Self {
            directory,
            workspace,
        }
    }
    fn path(&self) -> PathBuf {
        self.directory.path().join("form.hwpx")
    }
    fn replace(&self) -> Result<String> {
        self.workspace.replace_existing(
            "form.hwpx",
            b"edited bytes",
            &sha256(b"original document bytes"),
        )
    }
    fn original(&self) {
        assert_eq!(fs::read(self.path()).unwrap(), b"original document bytes");
        let names: Vec<_> = fs::read_dir(self.directory.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec!["form.hwpx"]);
    }
    fn no_temporary(&self) {
        assert!(!fs::read_dir(self.directory.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        reset_hooks();
    }
}
fn reset_hooks() {
    TEST_HOOK.with(|hook| *hook.borrow_mut() = None);
    TEST_INODE.with(|inode| inode.set(None));
    TEST_STABLE_INODES.with(|stable| stable.set(None));
    TEST_RANDOM.with(|random| *random.borrow_mut() = None);
    TEST_TIMESTAMP.with(|timestamp| *timestamp.borrow_mut() = None);
}
fn hook(action: impl FnMut(&str) -> Result<()> + 'static) {
    TEST_HOOK.with(|hook| *hook.borrow_mut() = Some(Box::new(action)));
}
fn fail_once(step: &'static str, code: &'static str) {
    let mut failed = false;
    hook(move |current| {
        if !failed && current == step {
            failed = true;
            return Err(WorkbenchError::new(
                code,
                format!("injected {step} failure"),
            ));
        }
        Ok(())
    });
}
#[test]
fn test_replace_reads_back_hash_and_keeps_mode() {
    let f = Fixture::new();
    assert_eq!(f.replace().unwrap(), sha256(b"edited bytes"));
    assert_eq!(fs::read(f.path()).unwrap(), b"edited bytes");
    assert_eq!(fs::metadata(f.path()).unwrap().mode() & 0o777, 0o640);
    f.no_temporary();
}
#[test]
fn test_changed_source_is_refused() {
    let f = Fixture::new();
    fs::write(f.path(), b"external edit").unwrap();
    assert_eq!(f.replace().unwrap_err().code, "SOURCE_CHANGED");
    assert_eq!(fs::read(f.path()).unwrap(), b"external edit");
    f.no_temporary();
}
#[test]
fn test_symlink_is_refused() {
    let f = Fixture::new();
    let target = f.directory.path().join("target.hwpx");
    fs::rename(f.path(), &target).unwrap();
    symlink(&target, f.path()).unwrap();
    assert_eq!(f.replace().unwrap_err().code, "UNSAFE_PATH");
    assert_eq!(fs::read(target).unwrap(), b"original document bytes");
    assert!(f.path().is_symlink());
}
#[test]
fn test_read_only_is_refused() {
    let f = Fixture::new();
    fs::set_permissions(f.path(), fs::Permissions::from_mode(0o400)).unwrap();
    assert_eq!(f.replace().unwrap_err().code, "READ_ONLY_DOCUMENT");
    f.original();
}
#[test]
fn test_nonregular_is_refused_without_blocking() {
    let f = Fixture::new();
    fs::remove_file(f.path()).unwrap();
    assert!(std::process::Command::new("mkfifo")
        .arg(f.path())
        .status()
        .unwrap()
        .success());
    assert_eq!(f.replace().unwrap_err().code, "UNSAFE_PATH");
}
#[test]
fn test_rename_failure_keeps_original_and_removes_temporary_files() {
    let f = Fixture::new();
    fail_once("publish", "WRITE_FAILED");
    assert_eq!(f.replace().unwrap_err().code, "WRITE_FAILED");
    f.original();
}
#[test]
fn test_directory_sync_failure_after_rename_restores_original() {
    let f = Fixture::new();
    fail_once("publication_sync", "WRITE_FAILED");
    assert_eq!(f.replace().unwrap_err().code, "WRITE_FAILED");
    f.original();
}
#[test]
fn test_temporary_collision_does_not_remove_existing_file() {
    let f = Fixture::new();
    let name = format!(".hwp-workbench-{}.tmp", "a".repeat(24));
    fs::write(
        f.directory.path().join(&name),
        b"belongs to another operation",
    )
    .unwrap();
    TEST_RANDOM.with(|r| *r.borrow_mut() = Some("a".repeat(24)));
    assert_eq!(f.replace().unwrap_err().code, "WRITE_FAILED");
    assert_eq!(fs::read(f.path()).unwrap(), b"original document bytes");
    assert_eq!(
        fs::read(f.directory.path().join(name)).unwrap(),
        b"belongs to another operation"
    );
}
#[test]
fn test_shared_backup_folder_isolates_same_named_sources() {
    let f = Fixture::new();
    fs::create_dir(f.directory.path().join("backups")).unwrap();
    let legacy = f
        .directory
        .path()
        .join("backups/form.20260101T000000Z-abcdef.hwpx");
    fs::write(&legacy, b"legacy previous version").unwrap();
    for source in ["a/form.hwpx", "b/form.hwpx"] {
        let mut saved = vec![];
        for revision in 0..12 {
            let bytes = format!("{source} revision {revision}").into_bytes();
            let name = f
                .workspace
                .backup_existing(source, &bytes, Some("backups"))
                .unwrap();
            saved.push((name, bytes));
        }
        let kept: Vec<_> = saved
            .into_iter()
            .filter(|(name, _)| f.directory.path().join(name).exists())
            .collect();
        assert_eq!(kept.len(), 10);
        for (name, bytes) in kept {
            assert_eq!(fs::read(f.directory.path().join(name)).unwrap(), bytes);
        }
    }
    assert_eq!(fs::read(legacy).unwrap(), b"legacy previous version");
}
#[test]
fn test_backup_bytes_name_and_private_modes() {
    let f = Fixture::new();
    let backup = f
        .workspace
        .backup_existing("form.hwpx", b"original document bytes", None)
        .unwrap();
    let path = f.directory.path().join(&backup);
    assert_eq!(fs::read(&path).unwrap(), b"original document bytes");
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert_eq!(
        fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(path.parent().unwrap().parent().unwrap())
            .unwrap()
            .mode()
            & 0o777,
        0o700
    );
    let identity = source_identity(&f.workspace.root, "form.hwpx");
    assert!(matches_backup(
        path.file_name().unwrap().to_str().unwrap(),
        &format!("form.{identity}."),
        ".hwpx"
    ));
    assert_eq!(fs::read(f.path()).unwrap(), b"original document bytes");
}
#[test]
fn test_retention_keeps_newest_ten_and_preserves_other_entries() {
    let f = Fixture::new();
    f.workspace.ensure_directory("backups").unwrap();
    let dir = f.directory.path().join("backups");
    let identity = source_identity(&f.workspace.root, "form.hwpx");
    let mut old = vec![];
    for i in 0..12 {
        let name = format!("form.{identity}.20260101T000000Z-{i:06x}.hwpx");
        let path = dir.join(&name);
        fs::write(&path, i.to_string()).unwrap();
        let file = File::open(&path).unwrap();
        file.set_times(
            fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_nanos(i + 1)),
        )
        .unwrap();
        old.push(name);
    }
    let unrelated = [
        "form.other.20260101T000000Z-000000.hwpx",
        "form.txt",
        "form.20260101T000000Z-000000.hwp",
        "form.20260101T000000Z-abcdefg.hwpx",
    ];
    for name in unrelated {
        fs::write(dir.join(name), b"keep").unwrap();
    }
    let linked = dir.join(format!("form.{identity}.20260101T000000Z-ffffff.hwpx"));
    symlink(f.path(), &linked).unwrap();
    let folder = dir.join(format!("form.{identity}.20260101T000000Z-eeeeee.hwpx"));
    fs::create_dir(&folder).unwrap();
    let newest = f
        .workspace
        .backup_existing("form.hwpx", b"original", Some("backups"))
        .unwrap();
    assert!(f.directory.path().join(newest).exists());
    for (i, name) in old.iter().enumerate() {
        assert_eq!(dir.join(name).exists(), i >= 3);
    }
    for name in unrelated {
        assert_eq!(fs::read(dir.join(name)).unwrap(), b"keep");
    }
    assert!(linked.is_symlink());
    assert!(folder.is_dir());
}
#[test]
fn test_symlink_backup_directory_is_refused() {
    let f = Fixture::new();
    fs::create_dir(f.directory.path().join("real-backups")).unwrap();
    symlink("real-backups", f.directory.path().join("backups")).unwrap();
    assert_eq!(
        f.workspace
            .backup_existing("form.hwpx", b"original", Some("backups"))
            .unwrap_err()
            .code,
        "UNSAFE_PATH"
    );
    assert_eq!(
        fs::read_dir(f.directory.path().join("real-backups"))
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn test_backup_directory_stays_inside_workspace() {
    let f = Fixture::new();
    assert_eq!(
        f.workspace
            .backup_existing("form.hwpx", b"original", Some("../backups"))
            .unwrap_err()
            .code,
        "OUTSIDE_WORKSPACE"
    );
}
#[test]
fn test_backup_collision_preserves_previous_file() {
    let f = Fixture::new();
    TEST_RANDOM.with(|r| *r.borrow_mut() = Some("abcdef".into()));
    TEST_TIMESTAMP.with(|t| *t.borrow_mut() = Some("20260101T000000Z".into()));
    let name = f
        .workspace
        .backup_existing("form.hwpx", b"original", Some("backups"))
        .unwrap();
    assert_eq!(
        f.workspace
            .backup_existing("form.hwpx", b"second", Some("backups"))
            .unwrap_err()
            .code,
        "OUTPUT_EXISTS"
    );
    assert_eq!(
        fs::read(f.directory.path().join(name)).unwrap(),
        b"original"
    );
    assert_eq!(
        fs::read_dir(f.directory.path().join("backups"))
            .unwrap()
            .count(),
        1
    );
}
#[test]
fn test_concurrent_writer_after_publish_keeps_revision_and_recovery() {
    for change in ["write", "replace", "same-bytes-replace"] {
        let f = Fixture::new();
        let path = f.path();
        hook(move |step| {
            if step == "after_publish" {
                let external = if change == "same-bytes-replace" {
                    b"edited bytes".as_slice()
                } else {
                    b"external edit"
                };
                if change == "write" {
                    fs::write(&path, external).unwrap();
                } else {
                    let writer = path.with_file_name("external.tmp");
                    fs::write(&writer, external).unwrap();
                    fs::rename(&writer, &path).unwrap();
                }
            }
            Ok(())
        });
        let error = f.replace().unwrap_err();
        assert_eq!(error.code, "SOURCE_CHANGED");
        let external = if change == "same-bytes-replace" {
            b"edited bytes".as_slice()
        } else {
            b"external edit"
        };
        assert_eq!(fs::read(f.path()).unwrap(), external);
        let backups = f.directory.path().join(".hwp-workbench/backups");
        let recovery = fs::read_dir(backups)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(fs::read(&recovery).unwrap(), b"original document bytes");
        assert!(error.message.contains(
            recovery
                .strip_prefix(f.directory.path())
                .unwrap()
                .to_str()
                .unwrap()
        ));
        assert_eq!(fs::metadata(recovery).unwrap().mode() & 0o777, 0o600);
        f.no_temporary();
    }
}
#[test]
fn test_final_source_check_catches_change_during_temporary_write() {
    let f = Fixture::new();
    let path = f.path();
    hook(move |step| {
        if step == "recovery_written" {
            fs::write(&path, b"external edit during temporary write").unwrap();
        }
        Ok(())
    });
    assert_eq!(f.replace().unwrap_err().code, "SOURCE_CHANGED");
    assert_eq!(
        fs::read(f.path()).unwrap(),
        b"external edit during temporary write"
    );
    f.no_temporary();
}
#[test]
fn test_readback_failure_restores_source_and_mode() {
    let f = Fixture::new();
    fail_once("readback", "VERIFY_FAILED");
    assert_eq!(f.replace().unwrap_err().code, "VERIFY_FAILED");
    f.original();
    assert_eq!(fs::metadata(f.path()).unwrap().mode() & 0o777, 0o640);
}
#[test]
fn test_temporary_fsync_failure_leaves_original_and_no_temporary_files() {
    let f = Fixture::new();
    fail_once("private_sync", "WRITE_FAILED");
    assert_eq!(f.replace().unwrap_err().code, "WRITE_FAILED");
    f.original();
}
#[test]
fn test_recovery_cleanup_failure_restores_original() {
    let f = Fixture::new();
    fail_once("recovery_cleanup", "WRITE_FAILED");
    assert_eq!(f.replace().unwrap_err().code, "WRITE_FAILED");
    f.original();
}
#[test]
fn test_read() {
    let f = Fixture::new();
    assert_eq!(
        f.workspace.read("form.hwpx").unwrap(),
        b"original document bytes"
    );
}
#[test]
fn test_outside_root() {
    let f = Fixture::new();
    assert_eq!(
        f.workspace.read("../form.hwpx").unwrap_err().code,
        "OUTSIDE_WORKSPACE"
    );
    assert_eq!(
        f.workspace.read("/etc/passwd").unwrap_err().code,
        "OUTSIDE_WORKSPACE"
    );
}
#[test]
fn test_symlink_file() {
    let f = Fixture::new();
    symlink("form.hwpx", f.directory.path().join("link")).unwrap();
    assert_eq!(f.workspace.read("link").unwrap_err().code, "READ_FAILED");
}
#[test]
fn test_symlink_directory() {
    let f = Fixture::new();
    fs::create_dir(f.directory.path().join("sub")).unwrap();
    symlink("sub", f.directory.path().join("link")).unwrap();
    assert_eq!(
        f.workspace
            .write_new("link/evil", b"test")
            .unwrap_err()
            .code,
        "UNSAFE_PATH"
    );
}
#[test]
fn test_write_exclusive() {
    let f = Fixture::new();
    f.workspace.write_new("new.txt", b"first").unwrap();
    assert_eq!(
        f.workspace
            .write_new("new.txt", b"second")
            .unwrap_err()
            .code,
        "OUTPUT_EXISTS"
    );
    assert_eq!(
        fs::read(f.directory.path().join("new.txt")).unwrap(),
        b"first"
    );
}
#[test]
fn test_private_output_mode() {
    let f = Fixture::new();
    f.workspace.write_new("new.txt", b"x").unwrap();
    assert_eq!(
        fs::metadata(f.directory.path().join("new.txt"))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
}
#[test]
fn directories_listing_read_limits_and_missing_paths() {
    let f = Fixture::new();
    f.workspace.ensure_directory("nested/deep").unwrap();
    f.workspace
        .write_new("nested/deep/yes.json", b"data")
        .unwrap();
    f.workspace.write_new("nested/deep/no.txt", b"x").unwrap();
    assert_eq!(
        f.workspace
            .list_files("nested/deep", |name| name.ends_with(".json"), 200)
            .unwrap(),
        vec!["yes.json"]
    );
    assert_eq!(
        f.workspace.list_files("missing", |_| true, 200).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        f.workspace
            .read_with_options("missing/file", 2, true)
            .unwrap(),
        None
    );
    assert_eq!(
        f.workspace
            .read_with_options("nested/deep/yes.json", 2, false)
            .unwrap_err()
            .message,
        "File exceeds 2 bytes."
    );
    for (path, code) in [
        ("", "BAD_PATH"),
        ("a\0b", "BAD_PATH"),
        (".", "OUTSIDE_WORKSPACE"),
        ("a/../b", "OUTSIDE_WORKSPACE"),
    ] {
        assert_eq!(f.workspace.relative_path(path).unwrap_err().code, code);
    }
    assert_eq!(f.workspace.relative_path("./a//b/./c").unwrap(), "a/b/c");
}
#[test]
fn external_revision_at_publication_is_restored_and_retained() {
    let f = Fixture::new();
    let path = f.path();
    hook(move |step| {
        if step == "publish" {
            fs::write(&path, b"external at publication").unwrap();
        }
        Ok(())
    });
    assert_eq!(f.replace().unwrap_err().code, "SOURCE_CHANGED");
    assert_eq!(fs::read(f.path()).unwrap(), b"external at publication");
}
#[test]
fn rollback_failure_keeps_recovery_bytes() {
    let f = Fixture::new();
    hook(move |step| {
        if step == "publication_sync" || step == "rollback" {
            return Err(write_error("injected failure"));
        }
        Ok(())
    });
    let error = f.replace().unwrap_err();
    assert_eq!(error.code, "WRITE_FAILED");
    let files: Vec<_> = fs::read_dir(f.directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|s| s == "tmp"))
        .collect();
    assert!(!files.is_empty());
    assert!(files
        .iter()
        .any(|path| fs::read(path).unwrap() == b"original document bytes"));
}
#[test]
fn external_revision_at_rollback_is_kept() {
    let f = Fixture::new();
    let path = f.path();
    hook(move |step| {
        if step == "publication_sync" {
            return Err(write_error("injected failure"));
        }
        if step == "rollback" {
            fs::write(&path, b"external at rollback").unwrap();
        }
        Ok(())
    });
    assert_eq!(f.replace().unwrap_err().code, "SOURCE_CHANGED");
    assert_eq!(fs::read(f.path()).unwrap(), b"external at rollback");
}
#[test]
fn recovery_path_remains_valid_when_final_retention_sync_fails() {
    let f = Fixture::new();
    let path = f.path();
    let mut failed = false;
    hook(move |step| {
        if step == "after_publish" {
            fs::write(&path, b"external revision").unwrap();
        }
        if step == "retention_source_sync" && !failed {
            failed = true;
            return Err(write_error("injected final retention sync"));
        }
        Ok(())
    });
    let error = f.replace().unwrap_err();
    assert_eq!(error.code, "SOURCE_CHANGED");
    let locations = error
        .message
        .strip_prefix("Document changed after publication; recovery kept at ")
        .unwrap();
    for location in locations.split(", ") {
        let location = f.directory.path().join(location);
        assert!(location.exists(), "{}", location.display());
        assert_eq!(fs::read(location).unwrap(), b"original document bytes");
    }
}
#[test]
fn external_revision_between_rollback_check_and_exchange_is_kept() {
    let f = Fixture::new();
    let path = f.path();
    hook(move |step| {
        if step == "publication_sync" {
            return Err(write_error("injected failure"));
        }
        if step == "rollback_exchange" {
            fs::write(&path, b"external raced rollback").unwrap();
        }
        Ok(())
    });
    assert_eq!(f.replace().unwrap_err().code, "SOURCE_CHANGED");
    assert_eq!(fs::read(f.path()).unwrap(), b"external raced rollback");
}

#[test]
fn unsupported_exchange_publication_uses_checked_rename() {
    for code in ["ENOTSUP", "EOPNOTSUPP", "EINVAL"] {
        let f = Fixture::new();
        fail_once("publish", code);
        assert_eq!(f.replace().unwrap(), sha256(b"edited bytes"));
        assert_eq!(fs::read(f.path()).unwrap(), b"edited bytes");
        assert_eq!(fs::metadata(f.path()).unwrap().mode() & 0o777, 0o640);
        f.no_temporary();
    }
}

#[test]
fn unsupported_exchange_rollback_uses_checked_rename() {
    for publication_fallback in [false, true] {
        let f = Fixture::new();
        hook(move |step| match step {
            "publish" if publication_fallback => {
                Err(WorkbenchError::new("ENOTSUP", "unsupported swap"))
            }
            "publication_sync" => Err(write_error("injected sync failure")),
            "rollback_exchange" => Err(WorkbenchError::new("ENOTSUP", "unsupported swap")),
            _ => Ok(()),
        });
        assert_eq!(f.replace().unwrap_err().code, "WRITE_FAILED");
        f.original();
    }
}

#[test]
fn fallback_rechecks_source_and_rollback_ownership() {
    for point in ["publish", "rollback_exchange"] {
        let f = Fixture::new();
        let path = f.path();
        hook(move |step| {
            if step == "publication_sync" && point == "rollback_exchange" {
                return Err(write_error("injected sync failure"));
            }
            if step == point {
                fs::write(&path, b"external revision").unwrap();
                return Err(WorkbenchError::new("ENOTSUP", "unsupported swap"));
            }
            Ok(())
        });
        assert_eq!(f.replace().unwrap_err().code, "SOURCE_CHANGED");
        assert_eq!(fs::read(f.path()).unwrap(), b"external revision");
        f.no_temporary();
    }
}

#[test]
fn unsupported_exchange_restores_raced_rollback_revision() {
    let f = Fixture::new();
    let path = f.path();
    hook(move |step| {
        if step == "publication_sync" {
            return Err(write_error("injected sync failure"));
        }
        if step == "rollback_exchange" {
            fs::write(&path, b"external raced rollback").unwrap();
        }
        if step == "rollback_restore_exchange" {
            return Err(WorkbenchError::new("ENOTSUP", "unsupported swap"));
        }
        Ok(())
    });
    assert_eq!(f.replace().unwrap_err().code, "SOURCE_CHANGED");
    assert_eq!(fs::read(f.path()).unwrap(), b"external raced rollback");
    f.no_temporary();
}

#[test]
fn fallback_rollback_sync_failure_reports_existing_recovery() {
    let f = Fixture::new();
    hook(move |step| match step {
        "publish" | "rollback_exchange" => Err(WorkbenchError::new("ENOTSUP", "unsupported swap")),
        "publication_sync" | "fallback_rollback_sync" => Err(write_error("injected sync failure")),
        _ => Ok(()),
    });
    let error = f.replace().unwrap_err();
    assert_eq!(error.code, "WRITE_FAILED");
    assert_eq!(fs::read(f.path()).unwrap(), b"original document bytes");
    let location = error.message.split("recovery kept at ").last().unwrap();
    assert_eq!(
        fs::read(f.directory.path().join(location)).unwrap(),
        b"original document bytes"
    );
}

#[test]
fn fallback_restore_sync_failure_reports_existing_recovery() {
    let f = Fixture::new();
    let path = f.path();
    hook(move |step| {
        if step == "publication_sync" || step == "fallback_restore_sync" {
            return Err(write_error("injected sync failure"));
        }
        if step == "rollback_exchange" {
            fs::write(&path, b"external raced rollback").unwrap();
        }
        if step == "rollback_restore_exchange" {
            return Err(WorkbenchError::new("ENOTSUP", "unsupported swap"));
        }
        Ok(())
    });
    let error = f.replace().unwrap_err();
    assert_eq!(error.code, "WRITE_FAILED");
    assert_eq!(fs::read(f.path()).unwrap(), b"external raced rollback");
    let location = error.message.split("recovery kept at ").last().unwrap();
    assert_eq!(
        fs::read(f.directory.path().join(location)).unwrap(),
        b"external raced rollback"
    );
}

#[test]
fn rollback_refuses_nonregular_displaced_source_without_blocking() {
    for kind in ["directory", "fifo"] {
        let f = Fixture::new();
        let original_inode = fs::metadata(f.path()).unwrap().ino();
        let directory = f.directory.path().to_owned();
        hook(move |step| {
            if step == "publication_sync" {
                return Err(write_error("injected sync failure"));
            }
            if step == "rollback" {
                let captured = fs::read_dir(&directory)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| fs::metadata(path).unwrap().ino() == original_inode)
                    .unwrap();
                fs::remove_file(&captured).unwrap();
                if kind == "fifo" {
                    assert!(std::process::Command::new("mkfifo")
                        .arg(&captured)
                        .status()
                        .unwrap()
                        .success());
                } else {
                    fs::create_dir(&captured).unwrap();
                }
            }
            Ok(())
        });
        let error = f.replace().unwrap_err();
        assert_eq!(error.code, "WRITE_FAILED");
        assert!(error.message.starts_with("Only regular files are allowed."));
        let location = error.message.split("recovery kept at ").last().unwrap();
        assert_eq!(
            fs::read(f.directory.path().join(location)).unwrap(),
            b"original document bytes"
        );
    }
}

#[test]
fn unsupported_link_backup_publication_keeps_original_and_verified_backup() {
    let f = Fixture::new();
    fail_once("new_link", "ENOTSUP");
    let backup = f
        .workspace
        .backup_existing("form.hwpx", b"original document bytes", None)
        .unwrap();
    let backup = f.directory.path().join(backup);
    assert_eq!(fs::read(&backup).unwrap(), b"original document bytes");
    assert_eq!(fs::metadata(backup).unwrap().mode() & 0o777, 0o600);
    assert_eq!(fs::read(f.path()).unwrap(), b"original document bytes");
    f.no_temporary();
    assert_eq!(f.replace().unwrap(), sha256(b"edited bytes"));
}

#[test]
fn unsupported_link_write_new_uses_exclusive_copy() {
    for code in ["ENOTSUP", "EOPNOTSUPP", "EPERM", "EMLINK"] {
        let f = Fixture::new();
        fail_once("new_link", code);
        f.workspace
            .write_new("new.hwpx", b"new document bytes")
            .unwrap();
        let output = f.directory.path().join("new.hwpx");
        assert_eq!(fs::read(&output).unwrap(), b"new document bytes");
        assert_eq!(fs::metadata(output).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::read(f.path()).unwrap(), b"original document bytes");
        f.no_temporary();
    }
}

#[test]
fn unsupported_link_recovery_retention_keeps_external_revision() {
    for code in ["ENOTSUP", "EOPNOTSUPP", "EPERM", "EMLINK"] {
        let f = Fixture::new();
        let path = f.path();
        hook(move |step| match step {
            "publish" => Err(WorkbenchError::new("ENOTSUP", "unsupported swap")),
            "after_publish" => {
                fs::write(&path, b"external revision").unwrap();
                Ok(())
            }
            "retain_recovery" => Err(WorkbenchError::new(code, "unsupported link")),
            _ => Ok(()),
        });
        let error = f.replace().unwrap_err();
        assert_eq!(error.code, "SOURCE_CHANGED");
        assert_eq!(fs::read(f.path()).unwrap(), b"external revision");
        let location = error
            .message
            .strip_prefix("Document changed after publication; recovery kept at ")
            .unwrap();
        assert!(location.starts_with(".hwp-workbench/backups/form.hwpx.recovery-"));
        let backup = f.directory.path().join(location);
        assert_eq!(fs::read(&backup).unwrap(), b"original document bytes");
        assert_eq!(fs::metadata(backup).unwrap().mode() & 0o777, 0o600);
        f.no_temporary();
    }
}

#[test]
fn unsupported_link_copy_refuses_existing_files_and_symlinks() {
    for symlinked in [false, true] {
        let f = Fixture::new();
        let output = f.directory.path().join("new.hwpx");
        let target = f.path();
        hook(move |step| {
            if step == "new_link" {
                if symlinked {
                    symlink(&target, &output).unwrap();
                } else {
                    fs::write(&output, b"another writer").unwrap();
                }
                return Err(WorkbenchError::new("ENOTSUP", "unsupported link"));
            }
            Ok(())
        });
        assert_eq!(
            f.workspace
                .write_new("new.hwpx", b"new bytes")
                .unwrap_err()
                .code,
            "OUTPUT_EXISTS"
        );
        let output = f.directory.path().join("new.hwpx");
        if symlinked {
            assert!(output.is_symlink());
        } else {
            assert_eq!(fs::read(output).unwrap(), b"another writer");
        }
        assert_eq!(fs::read(f.path()).unwrap(), b"original document bytes");
        f.no_temporary();
    }
}

#[test]
fn unsupported_link_copy_failures_remove_only_created_output() {
    for point in [
        "copy_write",
        "copy_file_sync",
        "copy_readback",
        "new_directory_sync",
    ] {
        let f = Fixture::new();
        let output = f.directory.path().join("new.hwpx");
        hook(move |step| match step {
            "new_link" => Err(WorkbenchError::new("ENOTSUP", "unsupported link")),
            step if step == point => {
                if point == "copy_write" {
                    fs::write(&output, b"partial copy").unwrap();
                }
                Err(write_error("injected copy failure"))
            }
            _ => Ok(()),
        });
        assert_eq!(
            f.workspace
                .write_new("new.hwpx", b"new bytes")
                .unwrap_err()
                .code,
            "WRITE_FAILED"
        );
        f.original();
    }
}

#[test]
fn unsupported_link_copy_detects_corruption_before_success() {
    let f = Fixture::new();
    let output = f.directory.path().join("new.hwpx");
    hook(move |step| match step {
        "new_link" => Err(WorkbenchError::new("ENOTSUP", "unsupported link")),
        "copy_readback" => {
            fs::write(&output, b"corrupted copy").unwrap();
            Ok(())
        }
        _ => Ok(()),
    });
    assert_eq!(
        f.workspace
            .write_new("new.hwpx", b"new bytes")
            .unwrap_err()
            .code,
        "VERIFY_FAILED"
    );
    f.original();
}

#[test]
fn unsupported_link_copy_cleanup_preserves_replaced_destination() {
    for symlinked in [false, true] {
        let f = Fixture::new();
        let output = f.directory.path().join("new.hwpx");
        let target = f.path();
        hook(move |step| match step {
            "new_link" => Err(WorkbenchError::new("ENOTSUP", "unsupported link")),
            "copy_readback" => {
                if symlinked {
                    fs::remove_file(&output).unwrap();
                    symlink(&target, &output).unwrap();
                } else {
                    let replacement = output.with_file_name("other.tmp");
                    fs::write(&replacement, b"new bytes").unwrap();
                    fs::rename(replacement, &output).unwrap();
                }
                Ok(())
            }
            _ => Ok(()),
        });
        assert!(f.workspace.write_new("new.hwpx", b"new bytes").is_err());
        let output = f.directory.path().join("new.hwpx");
        if symlinked {
            assert!(output.is_symlink());
        } else {
            assert_eq!(fs::read(output).unwrap(), b"new bytes");
        }
        assert_eq!(fs::read(f.path()).unwrap(), b"original document bytes");
        f.no_temporary();
    }
}

#[test]
fn unsupported_link_recovery_copy_failure_reports_intact_private_source() {
    for point in ["copy_file_sync", "copy_readback", "retention_backup_sync"] {
        let f = Fixture::new();
        let path = f.path();
        hook(move |step| match step {
            "publish" => Err(WorkbenchError::new("ENOTSUP", "unsupported swap")),
            "after_publish" => {
                fs::write(&path, b"external revision").unwrap();
                Ok(())
            }
            "retain_recovery" => Err(WorkbenchError::new("ENOTSUP", "unsupported link")),
            step if step == point => Err(write_error("injected copy failure")),
            _ => Ok(()),
        });
        let error = f.replace().unwrap_err();
        assert_eq!(error.code, "SOURCE_CHANGED");
        let location = error
            .message
            .strip_prefix("Document changed after publication; recovery kept at ")
            .unwrap();
        assert!(location.ends_with(".tmp"));
        assert_eq!(
            fs::read(f.directory.path().join(location)).unwrap(),
            b"original document bytes"
        );
        assert_eq!(fs::read(f.path()).unwrap(), b"external revision");
        assert_eq!(
            fs::read_dir(f.directory.path().join(".hwp-workbench/backups"))
                .unwrap()
                .count(),
            0
        );
    }
}

#[test]
fn unsupported_link_recovery_cleanup_failure_reports_verified_backup() {
    for point in ["retention_cleanup", "retention_source_sync"] {
        let f = Fixture::new();
        let path = f.path();
        hook(move |step| match step {
            "publish" => Err(WorkbenchError::new("ENOTSUP", "unsupported swap")),
            "after_publish" => {
                fs::write(&path, b"external revision").unwrap();
                Ok(())
            }
            "retain_recovery" => Err(WorkbenchError::new("ENOTSUP", "unsupported link")),
            step if step == point => Err(write_error("injected cleanup failure")),
            _ => Ok(()),
        });
        let error = f.replace().unwrap_err();
        assert_eq!(error.code, "SOURCE_CHANGED");
        let location = error
            .message
            .strip_prefix("Document changed after publication; recovery kept at ")
            .unwrap();
        assert!(location.starts_with(".hwp-workbench/backups/"));
        assert_eq!(
            fs::read(f.directory.path().join(location)).unwrap(),
            b"original document bytes"
        );
        assert_eq!(fs::read(f.path()).unwrap(), b"external revision");
    }
}

#[test]
fn unsupported_link_other_errors_fail_closed() {
    for code in ["EACCES", "EIO", "ENOSPC", "EXDEV", "WRITE_FAILED"] {
        let f = Fixture::new();
        fail_once("new_link", code);
        assert_eq!(
            f.workspace
                .write_new("new.hwpx", b"new bytes")
                .unwrap_err()
                .code,
            code
        );
        f.original();
    }
}

#[test]
fn unsupported_link_retained_copy_is_independent_and_verified() {
    let f = Fixture::new();
    let (parent, _) = f.workspace.parent("form.hwpx", false).unwrap();
    let parent = parent.unwrap();
    let recovery = f
        .workspace
        .private_file(&parent, b"recovery bytes", 0o640)
        .unwrap();
    let original_inode = fs::metadata(f.directory.path().join(&recovery))
        .unwrap()
        .ino();
    fail_once("retain_recovery", "ENOTSUP");
    let backup = f
        .workspace
        .retain_recovery("form.hwpx", &parent, &recovery)
        .unwrap();
    let backup = f.directory.path().join(backup);
    assert_eq!(fs::read(&backup).unwrap(), b"recovery bytes");
    let info = fs::metadata(&backup).unwrap();
    assert_ne!(info.ino(), original_inode);
    assert_eq!(info.mode() & 0o777, 0o600);
    f.no_temporary();
}

#[test]
fn unsupported_link_retention_collision_keeps_both_existing_files() {
    for symlinked in [false, true] {
        let f = Fixture::new();
        let (parent, _) = f.workspace.parent("form.hwpx", false).unwrap();
        let parent = parent.unwrap();
        let recovery = f
            .workspace
            .private_file(&parent, b"recovery bytes", 0o600)
            .unwrap();
        f.workspace
            .ensure_directory(".hwp-workbench/backups")
            .unwrap();
        TEST_RANDOM.with(|r| *r.borrow_mut() = Some("abcdefabcdef".into()));
        TEST_TIMESTAMP.with(|t| *t.borrow_mut() = Some("20260101T000000Z".into()));
        let collision = f
            .directory
            .path()
            .join(".hwp-workbench/backups/form.hwpx.recovery-20260101T000000Z-abcdefabcdef");
        if symlinked {
            symlink(f.path(), &collision).unwrap();
        } else {
            fs::write(&collision, b"another operation").unwrap();
        }
        fail_once("retain_recovery", "ENOTSUP");
        assert_eq!(
            f.workspace
                .retain_recovery("form.hwpx", &parent, &recovery)
                .unwrap_err()
                .code,
            "WRITE_FAILED"
        );
        assert_eq!(
            fs::read(f.directory.path().join(recovery)).unwrap(),
            b"recovery bytes"
        );
        if symlinked {
            assert!(collision.is_symlink());
        } else {
            assert_eq!(fs::read(collision).unwrap(), b"another operation");
        }
        assert_eq!(fs::read(f.path()).unwrap(), b"original document bytes");
    }
}

#[test]
fn unsupported_link_retention_detects_corruption_and_keeps_source() {
    let f = Fixture::new();
    let (parent, _) = f.workspace.parent("form.hwpx", false).unwrap();
    let parent = parent.unwrap();
    let recovery = f
        .workspace
        .private_file(&parent, b"recovery bytes", 0o600)
        .unwrap();
    let directory = f.directory.path().join(".hwp-workbench/backups");
    let backups = directory.clone();
    hook(move |step| match step {
        "retain_recovery" => Err(WorkbenchError::new("ENOTSUP", "unsupported link")),
        "copy_readback" => {
            let backup = fs::read_dir(&backups)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            fs::write(backup, b"corrupt recovery").unwrap();
            Ok(())
        }
        _ => Ok(()),
    });
    assert_eq!(
        f.workspace
            .retain_recovery("form.hwpx", &parent, &recovery)
            .unwrap_err()
            .code,
        "VERIFY_FAILED"
    );
    assert_eq!(
        fs::read(f.directory.path().join(recovery)).unwrap(),
        b"recovery bytes"
    );
    assert_eq!(fs::read_dir(directory).unwrap().count(), 0);
}

#[test]
fn unsupported_link_retention_rechecks_source_before_cleanup() {
    let f = Fixture::new();
    let (parent, _) = f.workspace.parent("form.hwpx", false).unwrap();
    let parent = parent.unwrap();
    let recovery = f
        .workspace
        .private_file(&parent, b"recovery bytes", 0o600)
        .unwrap();
    let path = f.directory.path().join(&recovery);
    let target = path.clone();
    hook(move |step| match step {
        "retain_recovery" => Err(WorkbenchError::new("ENOTSUP", "unsupported link")),
        "retention_cleanup" => {
            let other = target.with_file_name("other.tmp");
            fs::write(&other, b"recovery bytes").unwrap();
            fs::rename(other, &target).unwrap();
            Ok(())
        }
        _ => Ok(()),
    });
    let error = f
        .workspace
        .retain_recovery("form.hwpx", &parent, &recovery)
        .unwrap_err();
    assert_eq!(error.code, "SOURCE_CHANGED");
    let location = error.details.unwrap()["recoveryPath"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        fs::read(f.directory.path().join(location)).unwrap(),
        b"recovery bytes"
    );
    assert_eq!(fs::read(path).unwrap(), b"recovery bytes");
}

#[test]
fn supported_link_recovery_retains_same_inode() {
    let f = Fixture::new();
    let (parent, _) = f.workspace.parent("form.hwpx", false).unwrap();
    let parent = parent.unwrap();
    let recovery = f
        .workspace
        .private_file(&parent, b"recovery bytes", 0o600)
        .unwrap();
    let original_inode = fs::metadata(f.directory.path().join(&recovery))
        .unwrap()
        .ino();
    let backup = f
        .workspace
        .retain_recovery("form.hwpx", &parent, &recovery)
        .unwrap();
    let backup = f.directory.path().join(backup);
    assert_eq!(fs::metadata(&backup).unwrap().ino(), original_inode);
    assert_eq!(fs::read(backup).unwrap(), b"recovery bytes");
    f.no_temporary();
}

// Model descriptor/path inode values synthesized anew on each FAT/exFAT lookup.
fn unstable_inodes() {
    TEST_STABLE_INODES.with(|stable| stable.set(Some(false)));
    TEST_INODE.with(|inode| inode.set(Some(1)));
}

#[test]
fn unstable_inode_backup_and_write_new_verify_bytes() {
    let f = Fixture::new();
    unstable_inodes();
    hook(|step| match step {
        "new_link" => Err(WorkbenchError::new("ENOTSUP", "unsupported link")),
        _ => Ok(()),
    });
    let backup = f
        .workspace
        .backup_existing("form.hwpx", b"original document bytes", None)
        .unwrap();
    assert_eq!(
        f.workspace.read(&backup).unwrap(),
        b"original document bytes"
    );
    f.workspace.write_new("new.hwpx", b"new bytes").unwrap();
    assert_eq!(f.workspace.read("new.hwpx").unwrap(), b"new bytes");
    assert_eq!(
        f.workspace.read("form.hwpx").unwrap(),
        b"original document bytes"
    );
    f.no_temporary();
}

#[test]
fn unstable_inode_replace_and_rollback_keep_original() {
    for fail in [false, true] {
        let f = Fixture::new();
        unstable_inodes();
        hook(move |step| match step {
            "publish" | "rollback_exchange" => {
                Err(WorkbenchError::new("ENOTSUP", "unsupported exchange"))
            }
            "readback" if fail => Err(WorkbenchError::new(
                "VERIFY_FAILED",
                "injected readback failure",
            )),
            _ => Ok(()),
        });
        if fail {
            assert_eq!(f.replace().unwrap_err().code, "VERIFY_FAILED");
            f.original();
        } else {
            assert_eq!(f.replace().unwrap(), sha256(b"edited bytes"));
            assert_eq!(fs::read(f.path()).unwrap(), b"edited bytes");
            f.no_temporary();
        }
    }
}

#[test]
fn unstable_inode_recovery_retention_keeps_external_revision() {
    let f = Fixture::new();
    unstable_inodes();
    let path = f.path();
    hook(move |step| match step {
        "publish" | "retain_recovery" => Err(WorkbenchError::new(
            "ENOTSUP",
            "unsupported volume operation",
        )),
        "after_publish" => {
            fs::write(&path, b"external revision").unwrap();
            Ok(())
        }
        _ => Ok(()),
    });
    let error = f.replace().unwrap_err();
    assert_eq!(error.code, "SOURCE_CHANGED");
    assert_eq!(fs::read(f.path()).unwrap(), b"external revision");
    let recovery = error
        .message
        .strip_prefix("Document changed after publication; recovery kept at ")
        .unwrap();
    assert!(recovery.starts_with(".hwp-workbench/backups/"));
    assert_eq!(
        f.workspace.read(recovery).unwrap(),
        b"original document bytes"
    );
    f.no_temporary();
}

#[test]
fn stable_volume_rejects_changing_inode_numbers() {
    let f = Fixture::new();
    TEST_STABLE_INODES.with(|stable| stable.set(Some(true)));
    TEST_INODE.with(|inode| inode.set(Some(1)));
    assert_eq!(f.replace().unwrap_err().code, "SOURCE_CHANGED");
    f.original();
}

#[test]
fn unstable_inode_identity_checks_revision_fields_and_hash() {
    let f = Fixture::new();
    unstable_inodes();
    let mut file = File::open(f.path()).unwrap();
    let (_, original) = read_identity(&mut file).unwrap();
    file.rewind().unwrap();
    let (_, again) = read_identity(&mut file).unwrap();
    assert_ne!(original.inode, again.inode);
    assert!(same_file(&original, &again));
    // A same-size external edit can preserve mtime; descriptor-read bytes bind it.
    fs::write(f.path(), b"external document bytes").unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(original.info.modified().unwrap()))
        .unwrap();
    file.rewind().unwrap();
    let (_, changed_bytes) = read_identity(&mut file).unwrap();
    assert!(same_revision(&original.info, &changed_bytes.info));
    assert_ne!(original.hash, changed_bytes.hash);
    assert!(!same_file(&original, &changed_bytes));
    fs::write(f.path(), b"original document bytes").unwrap();
    let changed_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    file.set_times(std::fs::FileTimes::new().set_modified(changed_time))
        .unwrap();
    file.rewind().unwrap();
    let (_, changed) = read_identity(&mut file).unwrap();
    assert_eq!(original.hash, changed.hash);
    assert!(!same_file(&original, &changed));
    fs::write(f.path(), b"shorter").unwrap();
    file.rewind().unwrap();
    let (_, shorter) = read_identity(&mut file).unwrap();
    assert!(!same_file(&original, &shorter));
}

#[test]
fn unstable_inode_copy_failure_cleans_owned_partial_file() {
    let f = Fixture::new();
    unstable_inodes();
    let output = f.directory.path().join("new.hwpx");
    hook(move |step| match step {
        "new_link" => Err(WorkbenchError::new("ENOTSUP", "unsupported link")),
        "copy_file_sync" => {
            fs::write(&output, b"partial").unwrap();
            Err(write_error("injected copy failure"))
        }
        _ => Ok(()),
    });
    assert_eq!(
        f.workspace
            .write_new("new.hwpx", b"new bytes")
            .unwrap_err()
            .code,
        "WRITE_FAILED"
    );
    f.original();
}

#[test]
fn unstable_inode_copy_cleanup_keeps_replaced_destination() {
    let f = Fixture::new();
    unstable_inodes();
    let output = f.directory.path().join("new.hwpx");
    hook(move |step| match step {
        "new_link" => Err(WorkbenchError::new("ENOTSUP", "unsupported link")),
        "copy_readback" => {
            let replacement = output.with_file_name("external.tmp");
            fs::write(&replacement, b"new bytes").unwrap();
            // Use a distinct FAT-representable timestamp even on a fast test host.
            File::open(&replacement)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(
                    std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
                ))
                .unwrap();
            fs::rename(replacement, &output).unwrap();
            Ok(())
        }
        _ => Ok(()),
    });
    assert_eq!(
        f.workspace
            .write_new("new.hwpx", b"new bytes")
            .unwrap_err()
            .code,
        "SOURCE_CHANGED"
    );
    assert_eq!(f.workspace.read("new.hwpx").unwrap(), b"new bytes");
    assert_eq!(
        f.workspace.read("form.hwpx").unwrap(),
        b"original document bytes"
    );
    f.no_temporary();
}

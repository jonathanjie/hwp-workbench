use super::*;
use hwp_core::document::{sha, Document};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

fn fixture(format: &str) -> Vec<u8> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/generated");
    fs::read(root.join(if format == "hwp" {
        "legacy-review.hwp"
    } else {
        "review-form.hwpx"
    }))
    .unwrap()
}

fn setup(format: &str) -> (TempDir, Vec<u8>, Value) {
    let folder = TempDir::new().unwrap();
    let original = fixture(format);
    let document = Document::new(&original, "form").unwrap();
    let node = document.nodes.first().unwrap();
    let patch = json!({"schemaVersion":1,"baseSha256":document.hash,"operations":[{
        "op":"set_text","nodeId":node.id,"expectedText":node.text,"text":"Saved Korean text 한글"}]});
    fs::write(folder.path().join(format!("form.{format}")), &original).unwrap();
    (
        folder,
        original,
        json!({"path":format!("form.{format}"),"patch":patch,"acknowledgeRisks":true}),
    )
}

#[test]
fn test_native_preview_verified_without_any_file_write() {
    for format in ["hwp", "hwpx"] {
        let (folder, original, mut args) = setup(format);
        let result = invoke(folder.path(), "preview", &args).unwrap();
        use base64::prelude::*;
        let bytes = BASE64_STANDARD
            .decode(result["bytesBase64"].as_str().unwrap())
            .unwrap();
        assert_eq!(result["sha256"], sha(&bytes));
        let document = Document::new(&bytes, "preview").unwrap();
        assert!(document
            .nodes
            .iter()
            .any(|node| node.text == "Saved Korean text 한글"));
        assert_eq!(
            fs::read(folder.path().join(format!("form.{format}"))).unwrap(),
            original
        );
        assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 1);
        assert!(!result["warnings"].as_array().unwrap().is_empty());
        args["patch"]["operations"][0]["expectedText"] = json!("wrong");
        assert_eq!(
            invoke(folder.path(), "preview", &args).unwrap_err().code,
            "STALE_TEXT"
        );
        let document = Document::new(&original, "source").unwrap();
        args["patch"]["operations"][0]["expectedText"] = json!(document.nodes[0].text);
        args["patch"]["baseSha256"] = json!("wrong");
        assert_eq!(
            invoke(folder.path(), "preview", &args).unwrap_err().code,
            "STALE_DOCUMENT"
        );
    }
}

#[test]
fn test_plan_no_output() {
    let (folder, _, args) = setup("hwpx");
    let result = invoke(folder.path(), "plan", &args).unwrap();
    assert_eq!(result["changes"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 1);
}

#[test]
fn test_changed_source_is_refused_before_parsing_new_bytes() {
    let (folder, _, args) = setup("hwpx");
    fs::write(
        folder.path().join("form.hwpx"),
        b"external invalid revision",
    )
    .unwrap();
    assert_eq!(
        invoke(folder.path(), "save", &args).unwrap_err().code,
        "SOURCE_CHANGED"
    );
    assert!(!folder.path().join(".hwp-workbench").exists());
}

#[test]
fn test_inspect_extract_validate_and_format_share_native_model() {
    let (folder, original, args) = setup("hwpx");
    let document = Document::new(&original, "form.hwpx").unwrap();
    assert_eq!(
        invoke(folder.path(), "inspect", &args).unwrap(),
        document.inspect()
    );
    let extracted = invoke(folder.path(), "extract", &args).unwrap();
    assert_eq!(
        extracted["text"],
        document
            .nodes
            .iter()
            .map(|node| node.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(
        invoke(folder.path(), "validate", &args).unwrap()["valid"],
        true
    );
    assert!(invoke(folder.path(), "format", &args).unwrap().is_object());
}

#[derive(Default)]
struct TestBackend {
    fault: Option<&'static str>,
    check: Option<Value>,
    revision_path: Option<std::path::PathBuf>,
}
impl OperationBackend for TestBackend {
    fn serialize(&self, document: &Document) -> Result<Vec<u8>> {
        match self.fault {
            Some("serialize") => Err(WorkbenchError::new(
                "VERIFY_FAILED",
                "injected verification failure",
            )),
            Some("format") => Ok(fixture(if document.format == "hwp" {
                "hwpx"
            } else {
                "hwp"
            })),
            _ => document.serialize(true),
        }
    }
    fn independent_check(&self, _bytes: &[u8], _document: &Document) -> Value {
        if let Some(path) = &self.revision_path {
            fs::write(path, b"external change").unwrap();
        }
        self.check.clone().unwrap_or_else(
            || json!({"engine":"test evidence","opened":true,"editedTextsFound":true,"missing":[]}),
        )
    }
    fn before_backup(&self, workspace: &hwp_core::workspace::Workspace, path: &str) -> Result<()> {
        if self.fault == Some("source") {
            fs::write(workspace.root().join(path), b"external change").unwrap();
        }
        Ok(())
    }
    fn backup(
        &self,
        workspace: &hwp_core::workspace::Workspace,
        path: &str,
        document: &Document,
        directory: Option<&str>,
    ) -> Result<String> {
        if self.fault == Some("backup") {
            return Err(WorkbenchError::new(
                "WRITE_FAILED",
                "injected backup failure",
            ));
        }
        let backup = workspace.backup_existing(path, &document.original, directory)?;
        if self.fault == Some("corrupt-backup") {
            fs::write(workspace.root().join(&backup), b"injected corrupted backup").unwrap();
            return Err(WorkbenchError::new(
                "VERIFY_FAILED",
                "Previous-version backup differs from the source.",
            ));
        }
        if self.fault == Some("after-backup") {
            fs::write(workspace.root().join(path), b"external edit after backup").unwrap();
        }
        Ok(backup)
    }
    fn replace(
        &self,
        workspace: &hwp_core::workspace::Workspace,
        path: &str,
        bytes: &[u8],
        source_hash: &str,
    ) -> Result<String> {
        if self.fault == Some("rename") {
            return Err(WorkbenchError::new(
                "WRITE_FAILED",
                "injected rename failure",
            ));
        }
        workspace.replace_existing(path, bytes, source_hash)
    }
    fn preview_limit(&self) -> usize {
        if self.fault == Some("preview-limit") {
            1
        } else {
            MAX_PREVIEW_BYTES
        }
    }
}

fn call(folder: &TempDir, method: &str, args: &Value, backend: &TestBackend) -> Result<Value> {
    invoke_with_backend(folder.path(), method, args, backend)
}

macro_rules! save_tests {
    ($module:ident, $format:literal) => {
        mod $module {
            use super::*;
            fn original(folder: &TempDir, expected: &[u8]) {
                assert_eq!(fs::read(folder.path().join(concat!("form.", $format))).unwrap(), expected);
                assert!(fs::read_dir(folder.path()).unwrap().all(|entry| !entry.unwrap().path().extension().is_some_and(|extension| extension == "tmp")));
            }
            #[test]
            fn test_save_replaces_verified_native_bytes_and_keeps_previous_version() {
                use std::os::unix::fs::PermissionsExt;
                let (folder, bytes, args) = setup($format);
                let source = folder.path().join(concat!("form.", $format));
                fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
                let result = call(&folder, "save", &args, &TestBackend::default()).unwrap();
                let saved = fs::read(&source).unwrap();
                assert_eq!(result["saved"], true);
                assert_eq!(result["previousSha256"], sha(&bytes));
                assert_eq!(result["sha256"], sha(&saved));
                assert_eq!(Document::new(&saved, "saved").unwrap().nodes[0].text, "Saved Korean text 한글");
                assert_eq!(fs::read(folder.path().join(result["backup"].as_str().unwrap())).unwrap(), bytes);
                assert_eq!(fs::metadata(source).unwrap().permissions().mode() & 0o777, 0o640);
            }
            #[test]
            fn test_verification_failure_before_backup_leaves_original() {
                let (folder, bytes, args) = setup($format);
                let backend = TestBackend { fault: Some("serialize"), ..Default::default() };
                assert_eq!(call(&folder,"save",&args,&backend).unwrap_err().code,"VERIFY_FAILED");
                original(&folder, &bytes);
                assert!(!folder.path().join(".hwp-workbench").exists());
            }
            #[test]
            fn test_independent_reopen_or_missing_text_blocks_save() {
                for check in [json!({"opened":false,"error":{"code":"FAILED"}}),json!({"opened":true,"editedTextsFound":false,"missing":["node"]}),json!({"opened":true,"editedTextsFound":false})] {
                    let (folder, bytes, args) = setup($format);
                    let backend = TestBackend { check: Some(check), ..Default::default() };
                    assert_eq!(call(&folder,"save",&args,&backend).unwrap_err().code,"VERIFY_FAILED");
                    original(&folder, &bytes);
                    assert!(!folder.path().join(".hwp-workbench").exists());
                }
            }
            #[test]
            fn test_unavailable_independent_check_is_reported() {
                let (folder, _, args) = setup($format);
                let check = json!({"available":false,"reason":"Parser unavailable in this test"});
                let backend = TestBackend { check: Some(check.clone()), ..Default::default() };
                assert_eq!(call(&folder,"save",&args,&backend).unwrap()["independentCheck"],check);
            }
            #[test]
            fn test_backup_failure_leaves_original() {
                let (folder, bytes, args) = setup($format);
                let backend = TestBackend { fault: Some("backup"), ..Default::default() };
                assert_eq!(call(&folder,"save",&args,&backend).unwrap_err().code,"WRITE_FAILED");
                original(&folder, &bytes);
            }
            #[test]
            fn test_rename_failure_after_backup_leaves_original_and_keeps_backup() {
                let (folder, bytes, args) = setup($format);
                let backend = TestBackend { fault: Some("rename"), ..Default::default() };
                assert_eq!(call(&folder,"save",&args,&backend).unwrap_err().code,"WRITE_FAILED");
                original(&folder, &bytes);
                let backup = fs::read_dir(folder.path().join(".hwp-workbench/backups")).unwrap().next().unwrap().unwrap();
                assert_eq!(fs::read(backup.path()).unwrap(), bytes);
            }
            #[test]
            fn test_file_changed_during_verification_is_refused() {
                let (folder, _, args) = setup($format);
                let backend = TestBackend { fault: Some("source"), ..Default::default() };
                assert_eq!(call(&folder,"save",&args,&backend).unwrap_err().code,"SOURCE_CHANGED");
                original(&folder, b"external change");
                assert!(!folder.path().join(".hwp-workbench").exists());
            }
            #[test]
            fn test_changed_file_after_read_refuses_stale_patch() {
                let (folder, bytes, args) = setup($format);
                let mut changed = Document::new(&bytes, "changed").unwrap();
                changed.patch(&args["patch"]).unwrap();
                let changed = changed.serialize(true).unwrap();
                fs::write(folder.path().join(concat!("form.", $format)), &changed).unwrap();
                assert_eq!(call(&folder,"save",&args,&TestBackend::default()).unwrap_err().code,"SOURCE_CHANGED");
                original(&folder, &changed);
            }
            #[test]
            fn test_hash_rechecked_after_backup() {
                let (folder, _, args) = setup($format);
                let backend = TestBackend { fault: Some("after-backup"), ..Default::default() };
                assert_eq!(call(&folder,"save",&args,&backend).unwrap_err().code,"SOURCE_CHANGED");
                original(&folder, b"external edit after backup");
            }
            #[test]
            fn test_wrong_native_extension_refused() {
                let (folder, bytes, mut args) = setup($format);
                fs::rename(folder.path().join(concat!("form.", $format)), folder.path().join("wrong.txt")).unwrap();
                args["path"] = json!("wrong.txt");
                assert_eq!(call(&folder,"save",&args,&TestBackend::default()).unwrap_err().code,"OUTPUT_FORMAT");
                assert_eq!(fs::read(folder.path().join("wrong.txt")).unwrap(),bytes);
            }
            #[test]
            fn test_risk_acknowledgement_required() {
                let (folder, bytes, mut args) = setup($format);
                args["acknowledgeRisks"] = json!(false);
                assert_eq!(call(&folder,"save",&args,&TestBackend::default()).unwrap_err().code,"RISK_ACK_REQUIRED");
                original(&folder, &bytes);
            }
            #[test]
            fn test_incomplete_patch_and_wrong_expected_text_refused() {
                let (folder, bytes, mut args) = setup($format);
                args["patch"]["operations"][0].as_object_mut().unwrap().remove("text");
                assert_eq!(call(&folder,"save",&args,&TestBackend::default()).unwrap_err().code,"BAD_PATCH");
                args["patch"]["operations"][0]["text"] = json!("new");
                args["patch"]["operations"][0]["expectedText"] = json!("wrong");
                assert_eq!(call(&folder,"save",&args,&TestBackend::default()).unwrap_err().code,"STALE_TEXT");
                original(&folder, &bytes);
            }
            #[test]
            fn test_save_refuses_read_only_document() {
                use std::os::unix::fs::PermissionsExt;
                let (folder, bytes, args) = setup($format);
                fs::set_permissions(folder.path().join(concat!("form.", $format)), fs::Permissions::from_mode(0o400)).unwrap();
                assert_eq!(call(&folder,"save",&args,&TestBackend::default()).unwrap_err().code,"READ_ONLY_DOCUMENT");
                original(&folder, &bytes);
            }
            #[test]
            fn test_save_refuses_symlink_document() {
                let (folder, bytes, args) = setup($format);
                fs::rename(folder.path().join(concat!("form.", $format)), folder.path().join("target")).unwrap();
                std::os::unix::fs::symlink("target",folder.path().join(concat!("form.", $format))).unwrap();
                assert_eq!(call(&folder,"save",&args,&TestBackend::default()).unwrap_err().code,"READ_FAILED");
                assert_eq!(fs::read(folder.path().join("target")).unwrap(),bytes);
            }
            #[test]
            fn test_save_refuses_symlink_backup_folder() {
                let (folder, bytes, args) = setup($format);
                fs::create_dir(folder.path().join("versions")).unwrap();
                std::os::unix::fs::symlink("versions",folder.path().join(".hwp-workbench")).unwrap();
                assert_eq!(call(&folder,"save",&args,&TestBackend::default()).unwrap_err().code,"UNSAFE_PATH");
                original(&folder, &bytes);
                assert_eq!(fs::read_dir(folder.path().join("versions")).unwrap().count(),0);
            }
            #[test]
            fn test_backup_verification_failure_after_backup_leaves_original() {
                let (folder, bytes, args) = setup($format);
                let backend = TestBackend { fault: Some("corrupt-backup"), ..Default::default() };
                assert_eq!(call(&folder,"save",&args,&backend).unwrap_err().code,"VERIFY_FAILED");
                original(&folder, &bytes);
            }
            #[test]
            fn test_writer_cannot_change_native_format() {
                let (folder, bytes, args) = setup($format);
                let backend = TestBackend { fault: Some("format"), ..Default::default() };
                assert_eq!(call(&folder,"save",&args,&backend).unwrap_err().code,"VERIFY_FAILED");
                original(&folder, &bytes);
            }
            #[test]
            fn test_empty_edit_remains_independently_unverified() {
                let (folder, _, mut args) = setup($format);
                args["patch"]["operations"][0]["text"] = json!("");
                let check = json!({"opened":true,"editedTextsFound":false,"missing":[],"unverified":["node"]});
                let backend = TestBackend { check: Some(check.clone()), ..Default::default() };
                let result = call(&folder,"save",&args,&backend).unwrap();
                assert_eq!(result["independentCheck"],check);
                assert!(Document::new(fs::read(folder.path().join(concat!("form.", $format))).unwrap(),"saved").unwrap().nodes.first().is_some_and(|node|node.text.is_empty()));
            }
            #[test]
            fn test_apply_still_refuses_overwrite_and_does_not_gate_independent_check() {
                let (folder, bytes, mut args) = setup($format);
                let check = json!({"opened":false,"error":{"code":"FAILED"}});
                let backend = TestBackend { check: Some(check.clone()), ..Default::default() };
                args["output"] = args["path"].clone();
                assert_eq!(call(&folder,"apply",&args,&backend).unwrap_err().code,"OUTPUT_EXISTS");
                args["output"] = json!(concat!("new.",$format));
                let result = call(&folder,"apply",&args,&backend).unwrap();
                assert_eq!(result["sourceUntouched"],true);
                assert_eq!(result["independentCheck"],check);
                original(&folder, &bytes);
            }
        }
    }
}
save_tests!(hwpx_save, "hwpx");
save_tests!(hwp_save, "hwp");

#[test]
fn test_preview_size_cap_and_rpc_only_advertising() {
    let (folder, _, args) = setup("hwpx");
    assert!(!capabilities()["operations"]
        .as_array()
        .unwrap()
        .contains(&json!("preview")));
    let backend = TestBackend {
        fault: Some("preview-limit"),
        ..Default::default()
    };
    assert_eq!(
        call(&folder, "preview", &args, &backend).unwrap_err().code,
        "LIMIT"
    );
}

#[test]
fn test_apply_preserves_source() {
    let (folder, bytes, mut args) = setup("hwpx");
    args["output"] = json!("edited.hwpx");
    let result = call(&folder, "apply", &args, &TestBackend::default()).unwrap();
    assert_eq!(result["reopenedAndVerified"], true);
    assert_eq!(fs::read(folder.path().join("form.hwpx")).unwrap(), bytes);
}
#[test]
fn test_apply_no_overwrite() {
    let (folder, bytes, mut args) = setup("hwpx");
    args["output"] = json!("form.hwpx");
    assert_eq!(
        call(&folder, "apply", &args, &TestBackend::default())
            .unwrap_err()
            .code,
        "OUTPUT_EXISTS"
    );
    assert_eq!(fs::read(folder.path().join("form.hwpx")).unwrap(), bytes);
}
#[test]
fn test_apply_requires_ack() {
    let (folder, _, mut args) = setup("hwpx");
    args["output"] = json!("new.hwpx");
    args["acknowledgeRisks"] = json!(false);
    assert_eq!(
        call(&folder, "apply", &args, &TestBackend::default())
            .unwrap_err()
            .code,
        "RISK_ACK_REQUIRED"
    );
    assert!(!folder.path().join("new.hwpx").exists());
}
#[test]
fn test_output_format_enforced() {
    let (folder, _, mut args) = setup("hwpx");
    args["output"] = json!("fake.hwp");
    assert_eq!(
        call(&folder, "apply", &args, &TestBackend::default())
            .unwrap_err()
            .code,
        "OUTPUT_FORMAT"
    );
}

#[test]
fn workspace_errors_keep_python_errno_and_leaf_name() {
    let (folder, _, args) = setup("hwpx");
    std::os::unix::fs::symlink("form.hwpx", folder.path().join("link.hwpx")).unwrap();
    let mut args = args;
    args["path"] = json!("link.hwpx");
    let error = invoke(folder.path(), "inspect", &args).unwrap_err();
    assert!(error.message.starts_with("[Errno "), "{}", error.message);
    assert!(
        error.message.ends_with(": 'link.hwpx'"),
        "{}",
        error.message
    );
}

#[test]
fn preview_size_bounds_keep_python_integer_types() {
    let (folder, bytes, _) = setup("hwpx");
    let package = hwp_core::package_io::ZipPackage::new(&bytes).unwrap();
    let document = Document::new(&bytes, "fixture").unwrap();
    let header = std::str::from_utf8(document.part("Contents/header.xml").unwrap()).unwrap();
    let updated = header
        .replace("height=\"2200\"", "height=\"100\"")
        .replace("height=\"1000\"", "height=\"100\"");
    let changed = std::collections::BTreeMap::from([(
        "Contents/header.xml".to_owned(),
        updated.into_bytes(),
    )]);
    let bytes = package.rewrite(&changed).unwrap();
    fs::write(folder.path().join("form.hwpx"), bytes).unwrap();
    let result = invoke(folder.path(), "inspect", &json!({"path":"form.hwpx"})).unwrap();
    let sizes: Vec<_> = result["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|node| node.get("style").and_then(|style| style.get("size")))
        .collect();
    assert!(
        sizes.iter().any(|size| size.as_i64() == Some(5)),
        "{sizes:?}"
    );
}

fn applied_check(check: Value) -> Value {
    let (folder, original, mut args) = setup("hwpx");
    args["output"] = json!("saved.hwpx");
    let backend = TestBackend {
        check: Some(check.clone()),
        ..Default::default()
    };
    let result = call(&folder, "apply", &args, &backend).unwrap();
    assert_eq!(fs::read(folder.path().join("form.hwpx")).unwrap(), original);
    assert_eq!(
        Document::new(fs::read(folder.path().join("saved.hwpx")).unwrap(), "saved")
            .unwrap()
            .nodes[0]
            .text,
        "Saved Korean text 한글"
    );
    result["independentCheck"].clone()
}
#[test]
fn test_apply_reports_confirmed_text() {
    let check = json!({"engine":"rhwp 0.8.7","opened":true,"pageCount":1,"editedTextsFound":true,"missing":[],"editMethods":{"node":"paragraphs"}});
    assert_eq!(applied_check(check.clone()), check);
}
#[test]
fn test_apply_reports_missing_text() {
    let check = json!({"engine":"rhwp 0.8.7","opened":true,"pageCount":1,"editedTextsFound":false,"missing":["node"],"editMethods":{"node":"missing"}});
    assert_eq!(applied_check(check.clone()), check);
}
#[test]
fn test_apply_reports_unavailable_parser() {
    let check = json!({"available":false,"reason":"Node.js is unavailable."});
    assert_eq!(applied_check(check.clone()), check);
}
#[test]
fn test_independent_failure_does_not_block_verified_save() {
    let check = json!({"engine":"rhwp 0.8.7","opened":false,"error":{"code":"RHWP_FAILED","message":"Cannot open."}});
    assert_eq!(applied_check(check.clone()), check);
}
#[test]
fn test_save_is_not_exposed_to_mcp() {
    assert!(!crate::mcp::tools()
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "hwp_save"));
    assert_eq!(
        capabilities()["safety"]["overwrite"],
        "save operation only, verified, with backup"
    );
}

#[test]
fn test_service_agent_operations_and_compact_inspect() {
    for format in ["hwp", "hwpx"] {
        let (folder, _, args) = setup(format);
        let path = args["path"].clone();
        let read = invoke(folder.path(), "read", &json!({"path":path,"limit":1})).unwrap();
        assert_eq!(read["limit"], 1);
        let search = invoke(
            folder.path(),
            "search",
            &json!({"path":path,"query":"검토"}),
        )
        .unwrap();
        assert!(!search["matches"].as_array().unwrap().is_empty());
        let replace = invoke(
            folder.path(),
            "replace",
            &json!({"path":path,"find":"검토","replaceWith":"확정"}),
        )
        .unwrap();
        assert!(!replace["changes"].as_array().unwrap().is_empty());
        let set = invoke(
            folder.path(),
            "set",
            &json!({"path":path,"nodeId":"s0:t0","text":"확정"}),
        )
        .unwrap();
        assert_eq!(set["patch"]["operations"].as_array().unwrap().len(), 1);
        let fill = invoke(
            folder.path(),
            "fill",
            &json!({"path":path,"fields":{"기관명":"새 기관"}}),
        )
        .unwrap();
        assert_eq!(
            !fill["filled"].as_array().unwrap().is_empty(),
            format == "hwpx"
        );
        let compact = invoke(
            folder.path(),
            "inspect",
            &json!({"path":path,"compact":true}),
        )
        .unwrap();
        assert!(compact.get("view").is_none());
        assert!(compact.get("parts").is_none());
        assert!(invoke(folder.path(), "inspect", &json!({"path":path}))
            .unwrap()
            .get("view")
            .is_some());
    }
}
#[test]
fn test_apply_through_replace_is_verified_and_never_overwrites() {
    for format in ["hwp", "hwpx"] {
        let (folder, original, args) = setup(format);
        let mut args = json!({"path":args["path"],"find":"검토","replaceWith":"확정","output":format!("new.{format}"),"acknowledgeRisks":true});
        let result = call(&folder, "replace", &args, &TestBackend::default()).unwrap();
        assert_eq!(result["reopenedAndVerified"], true);
        let bytes = fs::read(folder.path().join(format!("new.{format}"))).unwrap();
        assert!(Document::new(&bytes, "saved").unwrap().nodes[0]
            .text
            .contains("확정"));
        assert_eq!(
            call(&folder, "replace", &args, &TestBackend::default())
                .unwrap_err()
                .code,
            "OUTPUT_EXISTS"
        );
        assert_eq!(
            fs::read(folder.path().join(format!("new.{format}"))).unwrap(),
            bytes
        );
        assert_eq!(
            fs::read(folder.path().join(format!("form.{format}"))).unwrap(),
            original
        );
        args["output"] = json!(format!("no-ack.{format}"));
        args["acknowledgeRisks"] = json!(false);
        assert_eq!(
            call(&folder, "replace", &args, &TestBackend::default())
                .unwrap_err()
                .code,
            "RISK_ACK_REQUIRED"
        );
        assert!(!folder.path().join(format!("no-ack.{format}")).exists());
    }
}
#[test]
fn test_fill_and_set_apply_through_shared_publication() {
    for (format, method) in [("hwpx", "fill"), ("hwpx", "set"), ("hwp", "set")] {
        let (folder, _, base) = setup(format);
        let args = json!({"path":base["path"],"output":format!("saved.{format}"),"acknowledgeRisks":true,"fields":{"기관명":"새 기관"},"nodeId":"s0:t0","text":"확정"});
        let result = call(&folder, method, &args, &TestBackend::default()).unwrap();
        assert_eq!(result["sourceUntouched"], true);
        let document = Document::new(
            fs::read(folder.path().join(format!("saved.{format}"))).unwrap(),
            "saved",
        )
        .unwrap();
        assert_eq!(result["outputSha256"], document.hash);
        for change in result["changes"].as_array().unwrap() {
            assert_eq!(
                document
                    .node(change["nodeId"].as_str().unwrap())
                    .unwrap()
                    .text,
                change["after"]
            );
        }
        assert_eq!(
            call(&folder, method, &args, &TestBackend::default())
                .unwrap_err()
                .code,
            "OUTPUT_EXISTS"
        );
    }
}
#[test]
fn test_replace_refuses_changed_source_before_publication() {
    let (folder, _, _) = setup("hwpx");
    let args = json!({"path":"form.hwpx","find":"검토","replaceWith":"확정","output":"stale.hwpx","acknowledgeRisks":true});
    let backend = TestBackend {
        revision_path: Some(folder.path().join("form.hwpx")),
        ..Default::default()
    };
    assert_eq!(
        call(&folder, "replace", &args, &backend).unwrap_err().code,
        "STALE_DOCUMENT"
    );
    assert!(!folder.path().join("stale.hwpx").exists());
    assert_eq!(
        fs::read(folder.path().join("form.hwpx")).unwrap(),
        b"external change"
    );
}
#[test]
fn test_empty_preview_cannot_write_and_output_stays_confined() {
    let (folder, _, _) = setup("hwpx");
    let mut args = json!({"path":"form.hwpx","find":"not present","replaceWith":"x"});
    assert!(
        invoke(folder.path(), "replace", &args).unwrap()["patch"]["operations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    args["output"] = json!("empty.hwpx");
    args["acknowledgeRisks"] = json!(true);
    assert_eq!(
        call(&folder, "replace", &args, &TestBackend::default())
            .unwrap_err()
            .code,
        "BAD_PATCH"
    );
    assert!(!folder.path().join("empty.hwpx").exists());
    args["find"] = json!("검토");
    args["replaceWith"] = json!("확정");
    args["output"] = json!("../outside.hwpx");
    assert_eq!(
        call(&folder, "replace", &args, &TestBackend::default())
            .unwrap_err()
            .code,
        "OUTSIDE_WORKSPACE"
    );
}
#[test]
fn test_recovery_hints_for_stable_codes() {
    for (code, word) in [
        ("STALE_DOCUMENT", "read"),
        ("STALE_TEXT", "read"),
        ("HWP_LENGTH_CHANGE_BLOCKED", "UTF-16"),
        ("OUTPUT_EXISTS", "filename"),
    ] {
        let detail = WorkbenchError::new(code, "test").as_json();
        assert_eq!(detail["code"], code);
        assert!(detail["hint"].as_str().unwrap().contains(word));
        assert_eq!(detail["retryable"], true);
    }
    assert_eq!(
        WorkbenchError::new("VERIFY_FAILED", "test").as_json()["retryable"],
        false
    );
}

#[test]
fn format_refuses_non_string_selectors_with_the_reference_error() {
    let (folder, _, _) = setup("hwpx");
    for node_id in [json!(false), json!(42), json!({}), json!([])] {
        let error = invoke(
            folder.path(),
            "format",
            &json!({"path":"form.hwpx","nodeId":node_id}),
        )
        .unwrap_err();
        assert_eq!(error.code, "BAD_ARGUMENTS");
        assert_eq!(error.message, "Unknown text node.");
    }
}

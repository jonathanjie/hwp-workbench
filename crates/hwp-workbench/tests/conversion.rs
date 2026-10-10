//! Service conversion contracts use only authored fixtures and mock legacy import.
use base64::{engine::general_purpose::STANDARD, Engine};
use hwp_core::{errors::Result, workspace::Workspace};
use hwp_verify::WorkerCommand;
use hwp_workbench::convert_operation::{convert_with_backends, convert_with_worker, native_texts};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

const DOCX: &[u8] =
    include_bytes!("../../../tests/data/convert_fixtures-docx-3bf7c69f1761e6c0.docx");
const HWP: &[u8] =
    include_bytes!("../../../tests/data/fixtures-cfb_from_streams-6563504cc4fd99b3.hwp");

fn setup() -> (tempfile::TempDir, Workspace) {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("source.docx"), DOCX).unwrap();
    let workspace = Workspace::new(folder.path()).unwrap();
    (folder, workspace)
}
fn worker() -> WorkerCommand {
    WorkerCommand::new(
        env!("CARGO_BIN_EXE_hwp-workbench"),
        vec!["__rhwp-worker".into()],
    )
}
fn run(workspace: &Workspace, args: Value) -> Result<Value> {
    convert_with_worker(workspace, &args, &worker())
}
fn no_legacy(_: &[u8], _: &str, _: &str) -> Result<(Option<Vec<u8>>, Value)> {
    panic!("unexpected legacy import")
}

#[test]
fn test_dry_run_and_acknowledged_publication() {
    let (folder, workspace) = setup();
    for args in [
        json!({"path":"source.docx","to":"hwpx"}),
        json!({"path":"source.docx","to":"hwpx","output":"new.hwpx"}),
    ] {
        let report = run(&workspace, args).unwrap();
        assert_eq!(report["available"], true);
        assert_eq!(report["textCheck"]["matches"], true);
        assert_eq!(report["unsupported"], json!({}));
        assert!(!folder.path().join("new.hwpx").exists());
    }
    let report = run(
        &workspace,
        json!({"path":"source.docx","to":"hwpx","output":"new.hwpx","acknowledgeRisks":true}),
    )
    .unwrap();
    assert_eq!(report["output"], "new.hwpx");
    let output = workspace.read("new.hwpx").unwrap();
    assert_eq!(report["outputSha256"], hwp_core::document::sha(&output));
    assert_eq!(
        native_texts(&output).unwrap(),
        hwp_convert::read_docx(DOCX).unwrap().paragraph_texts()
    );
}

#[test]
fn test_native_routes_and_docx_output() {
    let (_folder, workspace) = setup();
    workspace
        .write_new(
            "source.hwpx",
            &hwp_convert::write_hwpx(&hwp_convert::read_docx(DOCX).unwrap()).unwrap(),
        )
        .unwrap();
    let report = run(
        &workspace,
        json!({"path":"source.hwpx","to":"docx","output":"reverse.docx","acknowledgeRisks":true}),
    )
    .unwrap();
    assert_eq!(report["textCheck"]["missingParagraphs"], json!([]));
    assert_eq!(
        hwp_convert::read_docx(&workspace.read("reverse.docx").unwrap())
            .unwrap()
            .paragraph_texts(),
        hwp_convert::read_docx(DOCX).unwrap().paragraph_texts()
    );
    let report = run(
        &workspace,
        json!({"path":"source.hwpx","to":"hwp","output":"native.hwp","acknowledgeRisks":true}),
    )
    .unwrap();
    assert_eq!(report["available"], true);
    assert!(report["engineReports"][0].get("lossReport").is_some());
    for (path, target) in [
        ("native.hwp", "hwpx"),
        ("native.hwp", "docx"),
        ("source.docx", "hwp"),
    ] {
        let report = run(&workspace, json!({"path":path,"to":target})).unwrap();
        assert_eq!(report["textCheck"]["performed"], true);
        assert_eq!(report["textCheck"]["missingParagraphs"], json!([]));
    }
}

#[test]
fn test_cli_and_mcp_share_conversion_contract() {
    let (folder, _) = setup();
    let cli = Command::new(env!("CARGO_BIN_EXE_hwp-workbench"))
        .args(["convert", "source.docx", "--to", "hwpx", "--root"])
        .arg(folder.path())
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_hwp-workbench"))
        .args(["mcp", "--root"])
        .arg(folder.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"hwp_convert","arguments":{"path":"source.docx","to":"hwpx"}}}),
    ];
    let mut input = child.stdin.take().unwrap();
    for message in messages {
        writeln!(input, "{message}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let last = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .last()
        .unwrap()
        .to_owned();
    let mcp: Value = serde_json::from_str(&last).unwrap();
    assert_eq!(mcp["result"]["isError"], false);
    assert_eq!(
        serde_json::from_slice::<Value>(&cli.stdout).unwrap(),
        mcp["result"]["structuredContent"]
    );
}

#[test]
fn test_no_overwrite_confinement_and_source_race() {
    let (folder, workspace) = setup();
    workspace.write_new("copy.hwpx", b"KEEP").unwrap();
    let args =
        json!({"path":"source.docx","to":"hwpx","output":"copy.hwpx","acknowledgeRisks":true});
    assert_eq!(
        run(&workspace, args.clone()).unwrap_err().code,
        "OUTPUT_EXISTS"
    );
    assert_eq!(workspace.read("copy.hwpx").unwrap(), b"KEEP");
    for (field, path, code) in [
        ("output", "../outside.hwpx", "OUTSIDE_WORKSPACE"),
        ("path", "../outside.docx", "OUTSIDE_WORKSPACE"),
        ("output", "wrong.docx", "OUTPUT_FORMAT"),
    ] {
        let mut request = args.clone();
        request[field] = json!(path);
        assert_eq!(run(&workspace, request).unwrap_err().code, code);
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(folder.path(), folder.path().join("linked")).unwrap();
        let mut request = args.clone();
        request["output"] = json!("linked/new.hwpx");
        assert_eq!(run(&workspace, request).unwrap_err().code, "UNSAFE_PATH");
    }
    let error = convert_with_backends(
        &workspace,
        &json!({"path":"source.docx","to":"hwp","output":"race.hwp","acknowledgeRisks":true}),
        |_, _| {
            std::fs::write(folder.path().join("source.docx"), b"changed").unwrap();
            Ok(json!({"bytesBase64":STANDARD.encode(HWP),"lossReport":{"losses":[]}}))
        },
        no_legacy,
    )
    .unwrap_err();
    assert_eq!(error.code, "STALE_DOCUMENT");
    assert!(!folder.path().join("race.hwp").exists());
}

#[test]
fn test_injected_rhwp_loss_is_preserved_in_report() {
    let (folder, workspace) = setup();
    let losses = json!({"losses":[{"feature":"equation","count":2,"message":"Injected loss"}]});
    let report = convert_with_backends(
        &workspace,
        &json!({"path":"source.docx","to":"hwp"}),
        |_, _| Ok(json!({"bytesBase64":STANDARD.encode(HWP),"lossReport":losses})),
        no_legacy,
    )
    .unwrap();
    assert_eq!(
        report["engineReports"].as_array().unwrap().last().unwrap()["lossReport"],
        losses
    );
    assert_eq!(report["textCheck"]["matches"], false);
    assert!(
        report["textCheck"]["missingParagraphCount"]
            .as_u64()
            .unwrap()
            > 0
            || report["textCheck"]["changedParagraphCount"]
                .as_u64()
                .unwrap()
                > 0
    );
    assert!(!folder.path().join("source.hwp").exists());
}

#[test]
fn test_unsupported_objects_are_counted_in_both_directions() {
    let (_folder, workspace) = setup();
    let docx = include_bytes!("../../../tests/data/convert_fixtures-docx-57a4ccf89fab7d9c.docx");
    workspace.write_new("unsupported.docx", docx).unwrap();
    let report = run(&workspace, json!({"path":"unsupported.docx","to":"hwpx"})).unwrap();
    assert_eq!(
        report["unsupported"],
        json!(hwp_convert::read_docx(docx).unwrap().unsupported)
    );
    for (feature, count) in [
        ("fields", 1),
        ("trackedChanges", 2),
        ("equations", 1),
        ("headers", 1),
        ("footers", 1),
        ("footnotes", 1),
        ("comments", 1),
        ("textBoxes", 1),
        ("shapes", 1),
    ] {
        assert_eq!(report["unsupported"][feature], count);
    }
    workspace
        .write_new(
            "unsupported.hwpx",
            include_bytes!("../../../tests/data/fixtures-make_hwpx-a6f57a394319cef9.hwpx"),
        )
        .unwrap();
    let report = run(&workspace, json!({"path":"unsupported.hwpx","to":"docx"})).unwrap();
    for feature in ["headers", "fields", "equations"] {
        assert_eq!(report["unsupported"][feature], 1);
    }
    assert!(
        report["textCheck"]["missingParagraphCount"]
            .as_u64()
            .unwrap()
            > 0
            || report["textCheck"]["changedParagraphCount"]
                .as_u64()
                .unwrap()
                > 0
    );
}

#[test]
fn test_optional_dependencies_missing_are_reported_without_writing() {
    let (folder, workspace) = setup();
    workspace
        .write_new("legacy.rtf", b"{\\rtf1 Hello}")
        .unwrap();
    let report = convert_with_backends(&workspace, &json!({"path":"legacy.rtf","to":"docx","output":"new.docx","acknowledgeRisks":true}), |_, _| panic!("unexpected native export"), |_, _, _| Ok((None,json!({"engine":"LibreOffice","available":false,"reason":"LibreOffice is not installed."})))).unwrap();
    assert_eq!(report["available"], false);
    assert_eq!(report["engineReports"][0]["available"], false);
    assert_eq!(report["textCheck"]["performed"], false);
    assert!(!folder.path().join("new.docx").exists());
    let report = convert_with_backends(
        &workspace,
        &json!({"path":"source.docx","to":"hwp"}),
        |_, _| Ok(json!({"available":false,"reason":"Test runtime absent"})),
        no_legacy,
    )
    .unwrap();
    assert_eq!(report["available"], false);
    assert_eq!(
        report["engineReports"].as_array().unwrap().last().unwrap()["reason"],
        "Test runtime absent"
    );
}

#[test]
fn test_same_format_copy_keeps_original_bytes() {
    let (_folder, workspace) = setup();
    for (source, target, bytes) in [("source.docx", "docx", DOCX), ("native.hwp", "hwp", HWP)] {
        if source != "source.docx" {
            workspace.write_new(source, bytes).unwrap();
        }
        let name = format!("same.{target}");
        let report = run(
            &workspace,
            json!({"path":source,"to":target,"output":name,"acknowledgeRisks":true}),
        )
        .unwrap();
        assert_eq!(workspace.read(&name).unwrap(), bytes);
        assert_eq!(report["textCheck"]["matches"], true);
        assert_eq!(report["unsupported"], json!({}));
    }
}

#[test]
fn test_legacy_routes_use_bounded_docx_intermediate() {
    let (_folder, workspace) = setup();
    let odt = include_bytes!("../../../tests/data/convert_fixtures-package-582bda1f2b683fa2.docx");
    let doc = include_bytes!("../../../tests/data/fixtures-cfb_from_streams-4ff326636825970b.hwp");
    for (source, payload) in [
        ("doc", doc.as_slice()),
        ("odt", odt.as_slice()),
        ("rtf", b"{\\rtf1 Hello}".as_slice()),
    ] {
        let path = format!("input.{source}");
        workspace.write_new(&path, payload).unwrap();
        for target in ["hwp", "hwpx", "docx"] {
            let report = convert_with_backends(
                &workspace,
                &json!({"path":path,"to":target}),
                |_, _| Ok(json!({"bytesBase64":STANDARD.encode(HWP),"lossReport":{"losses":[]}})),
                |_, from, to| {
                    assert_eq!(from, source);
                    assert_eq!(to, "docx");
                    Ok((
                        Some(DOCX.to_vec()),
                        json!({"available":true,"engine":"LibreOffice"}),
                    ))
                },
            )
            .unwrap();
            assert_eq!(report["route"][0], format!("{source} → docx (LibreOffice)"));
            assert_eq!(report["unsupported"]["legacyImportUnverified"], 1);
            assert!(report["textCheck"]["source"]
                .as_str()
                .unwrap()
                .contains("intermediate"));
        }
    }
}

#[test]
fn test_24000_distinct_single_character_runs_convert_in_seconds() {
    let (_folder, workspace) = setup();
    workspace
        .write_new(
            "runs.docx",
            include_bytes!("../../../tests/data/convert_fixtures-docx-240d3121c9ae56b8.docx"),
        )
        .unwrap();
    let started = rustix::time::clock_gettime(rustix::time::ClockId::ProcessCPUTime);
    let report = run(&workspace, json!({"path":"runs.docx","to":"hwpx"})).unwrap();
    let finished = rustix::time::clock_gettime(rustix::time::ClockId::ProcessCPUTime);
    // Match the CPU-time budget; scheduling delays are not CPU work.
    let elapsed = (finished.tv_sec - started.tv_sec) as f64
        + (finished.tv_nsec - started.tv_nsec) as f64 / 1_000_000_000.0;
    assert_eq!(report["available"], true);
    assert_eq!(report["textCheck"]["matches"], true);
    assert_eq!(report["unsupported"], json!({}));
    assert!(
        elapsed < 8.0,
        "24,000 style service conversion used {elapsed:.3} CPU seconds"
    );
}

#[test]
fn malformed_native_exports_fail_without_publishing() {
    let (folder, workspace) = setup();
    let args = json!({"path":"source.docx","to":"hwp","output":"bad.hwp","acknowledgeRisks":true});
    for (response, message) in [
        (json!({}), "Invalid or oversized rhwp export."),
        (
            json!({"bytesBase64":"eA=="}),
            "rhwp export omitted its loss report.",
        ),
        (
            json!({"bytesBase64":"%%%","lossReport":{}}),
            "Invalid rhwp export bytes.",
        ),
    ] {
        let error =
            convert_with_backends(&workspace, &args, |_, _| Ok(response.clone()), no_legacy)
                .unwrap_err();
        assert_eq!(error.code, "INDEPENDENT_PROTOCOL");
        assert_eq!(error.message, message);
        assert!(!folder.path().join("bad.hwp").exists());
    }
    let hwpx = hwp_convert::write_hwpx(&hwp_convert::read_docx(DOCX).unwrap()).unwrap();
    let error = convert_with_backends(
        &workspace,
        &args,
        |_, _| Ok(json!({"bytesBase64":STANDARD.encode(&hwpx),"lossReport":{}})),
        no_legacy,
    )
    .unwrap_err();
    assert_eq!(error.code, "VERIFY_FAILED");
    assert!(!folder.path().join("bad.hwp").exists());
}

#[test]
fn direct_mapper_packages_match_frozen_contract_bytes_and_member_order() {
    let snapshot: Value = serde_json::from_str(include_str!("data/mapper-packages.json")).unwrap();
    let (folder, workspace) = setup();
    for case in snapshot["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/data")
            .join(source);
        let bytes = std::fs::read(input).unwrap();
        assert_eq!(hwp_core::document::sha(&bytes), case["sourceSha256"]);
        std::fs::write(folder.path().join(source), bytes).unwrap();
        let target = case["target"].as_str().unwrap();
        let output = format!("{}.{}", source, target);
        run(
            &workspace,
            json!({"path":source,"to":target,"output":output,"acknowledgeRisks":true}),
        )
        .unwrap();
        let payload = workspace.read(&output).unwrap();
        let package = hwp_core::package_io::ZipPackage::new(&payload).unwrap();
        let names: Vec<_> = package.names().collect();
        let (parts, _) = hwp_convert::package::read_package(&payload).unwrap();
        let expected: Vec<_> = case["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|member| member["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, expected, "{source} member order");
        for member in case["members"].as_array().unwrap() {
            let name = member["name"].as_str().unwrap();
            let bytes = STANDARD
                .decode(member["bytesBase64"].as_str().unwrap())
                .unwrap();
            if parts[name] != bytes {
                let offset = parts[name]
                    .iter()
                    .zip(&bytes)
                    .position(|(left, right)| left != right)
                    .unwrap_or(parts[name].len().min(bytes.len()));
                let start = offset.saturating_sub(60);
                panic!(
                    "{source} member {name} differs at {offset}: actual {} expected {}",
                    String::from_utf8_lossy(
                        &parts[name][start..parts[name].len().min(offset + 120)]
                    ),
                    String::from_utf8_lossy(&bytes[start..bytes.len().min(offset + 120)])
                );
            }
        }
    }
}

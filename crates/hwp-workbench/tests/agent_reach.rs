use hwp_core::errors::WorkbenchError;
use hwp_core::workspace::Workspace;
use hwp_workbench::{
    cli,
    desktop_open::{self, DesktopEnvironment},
    mcp_config, skill_install,
};
use serde_json::{json, Value};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
fn workspace() -> tempfile::TempDir {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/generated/review-form.hwpx"),
        root.path().join("form.hwpx"),
    )
    .unwrap();
    root
}
fn cli(root: &Path, args: Vec<&str>, input: &str) -> Value {
    let mut argv = vec!["hwp-workbench"];
    argv.extend(args);
    argv.extend(["--root", root.to_str().unwrap()]);
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let code = cli::run(argv, input.as_bytes(), &mut output, &mut errors);
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&errors));
    serde_json::from_slice(&output).unwrap()
}
fn environment(root: &Path) -> DesktopEnvironment {
    let project = root.join("checkout");
    std::fs::create_dir_all(project.join("desktop")).unwrap();
    std::fs::write(project.join("package.json"), "{}").unwrap();
    std::fs::write(project.join("desktop/main.cjs"), "// source").unwrap();
    let directories = vec![root.join("Applications"), root.join("User Applications")];
    for directory in &directories {
        std::fs::create_dir(directory).unwrap();
    }
    DesktopEnvironment {
        macos: true,
        application_directories: directories,
        project,
    }
}
#[test]
fn test_preview_envelopes_plan_and_apply_in_cli_mcp_and_rpc() {
    let root = workspace();
    let inspect =
        hwp_workbench::service::invoke(root.path(), "inspect", &json!({"path":"form.hwpx"}))
            .unwrap();
    let id = &inspect["nodes"][0]["id"];
    for (index, (method, args)) in [
        ("replace", json!({"find":"검토","replaceWith":"확정"})),
        ("fill", json!({"fields":{"기관명":"New organisation"}})),
        ("set", json!({"nodeId":id,"text":"Reviewed text"})),
    ]
    .into_iter()
    .enumerate()
    {
        let mut args = args;
        args.as_object_mut()
            .unwrap()
            .insert("path".to_owned(), json!("form.hwpx"));
        let preview = hwp_workbench::service::invoke(root.path(), method, &args).unwrap();
        std::fs::write(root.path().join("preview.json"), preview.to_string()).unwrap();
        let expected = hwp_workbench::service::invoke(
            root.path(),
            "plan",
            &json!({"path":"form.hwpx","patch":preview["patch"]}),
        )
        .unwrap();
        for transport in ["cli", "rpc", "mcp"] {
            for method in ["plan", "apply"] {
                let output = format!("reviewed-{index}-{transport}.hwpx");
                let mut args = json!({"path":"form.hwpx","patch":preview});
                if method == "apply" {
                    args["output"] = json!(output);
                    args["acknowledgeRisks"] = json!(true);
                }
                let result = if transport == "rpc" {
                    cli(
                        root.path(),
                        vec!["rpc"],
                        &json!({"method":method,"args":args}).to_string(),
                    )
                } else if transport == "cli" {
                    let mut flags = vec![method, "form.hwpx", "--patch", "preview.json"];
                    if method == "apply" {
                        flags.extend(["-o", &output, "--acknowledge-risks"]);
                    }
                    cli(root.path(), flags, "")
                } else {
                    let input = format!(
                        "{}\n{}\n{}\n",
                        json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
                        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":format!("hwp_{method}"),"arguments":args}})
                    );
                    let mut output = Vec::new();
                    hwp_workbench::mcp::serve(
                        root.path(),
                        std::io::Cursor::new(input),
                        &mut output,
                    )
                    .unwrap();
                    let reply: Value = serde_json::from_str(
                        String::from_utf8(output).unwrap().lines().last().unwrap(),
                    )
                    .unwrap();
                    assert_eq!(reply["result"]["isError"], false, "{reply}");
                    reply["result"]["structuredContent"].clone()
                };
                assert_eq!(result["changes"], expected["changes"]);
                if method == "apply" {
                    assert_eq!(result["sourceUntouched"], true);
                }
            }
        }
    }
}
#[test]
fn test_envelope_keeps_complete_patch_validation_and_no_overwrite() {
    let root = workspace();
    let preview = hwp_workbench::service::invoke(
        root.path(),
        "set",
        &json!({"path":"form.hwpx","nodeId":"s0:t0","text":"Reviewed"}),
    )
    .unwrap();
    let mut stale = preview.clone();
    stale["patch"]["baseSha256"] = json!("0".repeat(64));
    let mut wrong = preview.clone();
    wrong["patch"]["operations"][0]["expectedText"] = json!("wrong");
    let mut duplicate = preview.clone();
    let operation = duplicate["patch"]["operations"][0].clone();
    duplicate["patch"]["operations"]
        .as_array_mut()
        .unwrap()
        .push(operation);
    for (patch, code) in [
        (stale, "STALE_DOCUMENT"),
        (wrong, "STALE_TEXT"),
        (duplicate, "BAD_PATCH"),
        (
            json!({"patch":{"schemaVersion":1,"baseSha256":preview["baseSha256"],"operations":[]}}),
            "BAD_PATCH",
        ),
        (
            json!({"patch":preview["patch"],"schemaVersion":2}),
            "BAD_PATCH",
        ),
    ] {
        let error=hwp_workbench::service::invoke(root.path(),"apply",&json!({"path":"form.hwpx","patch":patch,"output":"invalid.hwpx","acknowledgeRisks":true})).unwrap_err();
        assert_eq!(error.code, code);
        assert!(!root.path().join("invalid.hwpx").exists());
    }
    std::fs::write(root.path().join("existing.hwpx"), b"keep this output").unwrap();
    let error=hwp_workbench::service::invoke(root.path(),"apply",&json!({"path":"form.hwpx","patch":preview,"output":"existing.hwpx","acknowledgeRisks":true})).unwrap_err();
    assert_eq!(error.code, "OUTPUT_EXISTS");
    assert_eq!(
        std::fs::read(root.path().join("existing.hwpx")).unwrap(),
        b"keep this output"
    );
}
#[test]
fn test_mcp_config_prints_exact_setup_for_each_client() {
    let root = workspace();
    let documents = root.path().join("Document folder's name");
    std::fs::create_dir(&documents).unwrap();
    let launcher = Path::new(env!("CARGO_BIN_EXE_hwp-workbench"));
    let project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    for (client, prefix) in [
        (
            "claude-code",
            "claude mcp add hwp-workbench --scope user --",
        ),
        ("codex", "codex mcp add hwp-workbench --"),
    ] {
        let setup = mcp_config::setup(client, &documents).unwrap();
        assert_eq!(
            setup,
            format!(
                "{prefix} {} mcp --root '{}'",
                launcher.display(),
                documents.display().to_string().replace('\'', "'\"'\"'")
            )
        );
    }
    let configuration: Value =
        serde_json::from_str(&mcp_config::setup("json", &documents).unwrap()).unwrap();
    assert_eq!(
        configuration,
        json!({"mcpServers":{"hwp-workbench":{"command":launcher,"args":["mcp","--root",documents]}}})
    );
    assert_eq!(
        mcp_config::setup("claude-desktop", &documents).unwrap(),
        project.join("dist/hwp-workbench.mcpb").to_string_lossy()
    );
    assert_eq!(std::fs::read_dir(documents).unwrap().count(), 0);
}
#[test]
fn test_mcp_config_refuses_missing_file_home_and_filesystem_roots() {
    let root = workspace();
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let alias = root.path().join("home-alias");
    symlink(&home, &alias).unwrap();
    for path in [
        root.path().join("missing"),
        root.path().join("form.hwpx"),
        home,
        PathBuf::from("/"),
        alias,
    ] {
        assert_eq!(
            mcp_config::setup("json", path).unwrap_err().code,
            "BAD_ROOT"
        );
    }
}
#[test]
fn test_open_prefers_packaged_app_in_both_application_folders() {
    let root = workspace();
    let environment = environment(root.path());
    for folder in &environment.application_directories {
        let app = folder.join("HWP Workbench.app");
        std::fs::create_dir(&app).unwrap();
        let result =
            desktop_open::open_document_with(root.path(), "form.hwpx", &environment, |request| {
                assert_eq!(request.program, Path::new("open"));
                assert_eq!(
                    request.args,
                    vec![
                        "-a".to_owned(),
                        "HWP Workbench".to_owned(),
                        root.path().join("form.hwpx").to_string_lossy().into_owned()
                    ]
                );
                assert!(!request.detached);
                Ok(())
            })
            .unwrap();
        assert_eq!(result, json!({"opened":true,"via":"app"}));
        std::fs::remove_dir(app).unwrap();
    }
}
#[test]
fn test_open_source_electron_is_detached_and_uses_absolute_paths() {
    let root = workspace();
    let environment = environment(root.path());
    let electron = environment.project.join("node_modules/.bin/electron");
    std::fs::create_dir_all(electron.parent().unwrap()).unwrap();
    std::fs::write(&electron, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&electron, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = desktop_open::open_document_with(
        root.path(),
        root.path().join("form.hwpx").to_str().unwrap(),
        &environment,
        |request| {
            assert_eq!(request.program, electron);
            assert!(request.detached);
            assert_eq!(request.cwd.as_ref(), Some(&environment.project));
            assert_eq!(
                request.args,
                vec![
                    environment.project.to_string_lossy().into_owned(),
                    "--open".to_owned(),
                    root.path().join("form.hwpx").to_string_lossy().into_owned()
                ]
            );
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(result, json!({"opened":true,"via":"electron"}));
}
#[test]
fn test_open_rejects_unsafe_paths_before_launch() {
    let root = workspace();
    let environment = environment(root.path());
    std::fs::write(root.path().join("wrong.pdf"), b"document").unwrap();
    std::fs::create_dir(root.path().join("folder.hwpx")).unwrap();
    symlink(
        root.path().join("form.hwpx"),
        root.path().join("linked.hwpx"),
    )
    .unwrap();
    symlink(root.path(), root.path().join("linked-folder")).unwrap();
    for path in [
        "missing.hwpx",
        "wrong.pdf",
        "folder.hwpx",
        "linked.hwpx",
        "linked-folder/form.hwpx",
        "../outside.hwpx",
        "/outside.hwpx",
    ] {
        assert!(
            desktop_open::open_document_with(root.path(), path, &environment, |_| panic!(
                "unsafe input must not launch"
            ))
            .is_err()
        );
    }
}
#[test]
fn test_open_reports_missing_app_and_launch_failure_with_hints() {
    let root = workspace();
    let environment = environment(root.path());
    let error = desktop_open::open_document_with(root.path(), "form.hwpx", &environment, |_| {
        panic!("missing app must not launch")
    })
    .unwrap_err();
    assert_eq!(error.code, "APP_UNAVAILABLE");
    assert_eq!(error.as_json()["retryable"], true);
    std::fs::create_dir(environment.application_directories[0].join("HWP Workbench.app")).unwrap();
    let error = desktop_open::open_document_with(root.path(), "form.hwpx", &environment, |_| {
        Err(WorkbenchError::new(
            "OPEN_FAILED",
            "Could not launch HWP Workbench: injected failure",
        ))
    })
    .unwrap_err();
    assert_eq!(error.code, "OPEN_FAILED");
    assert!(error.as_json()["hint"].is_string());
}
#[test]
fn test_install_status_uninstall_and_path_substitution() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let workspace = Workspace::new(root.path()).unwrap();
    assert_eq!(
        skill_install::status(&workspace)["claude"]["installed"],
        false
    );
    skill_install::install(&workspace, "all", false).unwrap();
    for target in ["claude", "codex"] {
        let text = std::fs::read_to_string(
            root.path()
                .join(format!(".{target}/skills/hwp-workbench/SKILL.md")),
        )
        .unwrap();
        assert!(text.contains("<!-- installed by hwp-workbench 0.1.0-alpha.1 -->"));
        assert!(!text.contains("{{HWP_WORKBENCH}}"));
    }
    skill_install::install(&workspace, "all", false).unwrap();
    let result = skill_install::uninstall(&workspace, "all").unwrap();
    assert_eq!(result["removed"], json!(["claude", "codex"]));
    assert_eq!(result["codex"]["installed"], false);
}
#[test]
fn test_dry_run_and_single_target() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let workspace = Workspace::new(root.path()).unwrap();
    skill_install::install(&workspace, "all", true).unwrap();
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    skill_install::install(&workspace, "claude", false).unwrap();
    assert!(root
        .path()
        .join(".claude/skills/hwp-workbench/SKILL.md")
        .is_file());
    assert!(!root.path().join(".codex").exists());
}
#[test]
fn test_unmarked_folder_refused_and_preserved_on_uninstall() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let workspace = Workspace::new(root.path()).unwrap();
    let folder = root.path().join(".codex/skills/hwp-workbench");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("SKILL.md"), "User-owned skill").unwrap();
    for dry in [false, true] {
        assert_eq!(
            skill_install::install(&workspace, "all", dry)
                .unwrap_err()
                .code,
            "SKILL_NOT_OWNED"
        );
        assert!(!root.path().join(".claude").exists());
    }
    skill_install::uninstall(&workspace, "all").unwrap();
    assert_eq!(
        std::fs::read_to_string(folder.join("SKILL.md")).unwrap(),
        "User-owned skill"
    );
}
#[test]
fn test_symlink_targets_and_marker_files_are_not_followed() {
    let home = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let external = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let workspace = Workspace::new(home.path()).unwrap();
    symlink(external.path(), home.path().join(".claude")).unwrap();
    assert_eq!(
        skill_install::install(&workspace, "claude", false)
            .unwrap_err()
            .code,
        "UNSAFE_PATH"
    );
    skill_install::uninstall(&workspace, "all").unwrap();
    assert_eq!(std::fs::read_dir(external.path()).unwrap().count(), 0);
    std::fs::remove_file(home.path().join(".claude")).unwrap();
    let folder = home.path().join(".claude/skills/hwp-workbench");
    std::fs::create_dir_all(&folder).unwrap();
    let marker = external.path().join("marker.md");
    std::fs::write(
        &marker,
        "<!-- installed by hwp-workbench 0.1.0-alpha.1 -->\nExternal",
    )
    .unwrap();
    symlink(&marker, folder.join("SKILL.md")).unwrap();
    assert_eq!(
        skill_install::install(&workspace, "claude", false)
            .unwrap_err()
            .code,
        "SKILL_NOT_OWNED"
    );
    skill_install::uninstall(&workspace, "all").unwrap();
    assert!(marker.is_file());
}
#[test]
fn test_marked_skill_is_backed_up_before_replacement() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let workspace = Workspace::new(root.path()).unwrap();
    skill_install::install(&workspace, "codex", false).unwrap();
    std::fs::write(
        root.path().join(".codex/skills/hwp-workbench/local.conf"),
        "user settings",
    )
    .unwrap();
    let result = skill_install::install(&workspace, "codex", false).unwrap();
    let backup = result["backups"][0].as_str().unwrap();
    assert_eq!(
        std::fs::read_to_string(Path::new(backup).join("local.conf")).unwrap(),
        "user settings"
    );
}
fn mcp(root: &Path, method: &str, args: Value) -> Value {
    let input = format!(
        "{}\n{}\n{}\n",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":format!("hwp_{method}"),"arguments":args}})
    );
    let mut output = Vec::new();
    hwp_workbench::mcp::serve(root, std::io::Cursor::new(input), &mut output).unwrap();
    let reply: Value =
        serde_json::from_str(String::from_utf8(output).unwrap().lines().last().unwrap()).unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    reply["result"]["structuredContent"].clone()
}
#[test]
fn test_short_references_cli_mcp_and_patch_operations() {
    let root = workspace();
    let read =
        hwp_workbench::service::invoke(root.path(), "read", &json!({"path":"form.hwpx","limit":1}))
            .unwrap();
    let native = &read["references"]["s0:t0"];
    let preview = cli(
        root.path(),
        vec![
            "set",
            "form.hwpx",
            "--node-id",
            "s0:t0",
            "--text",
            "Reviewed",
        ],
        "",
    );
    assert_eq!(preview["patch"]["operations"][0]["nodeId"], *native);
    assert_eq!(
        mcp(
            root.path(),
            "set",
            json!({"path":"form.hwpx","nodeId":"s0:t0","text":"Reviewed"})
        ),
        preview
    );
    let replacement = cli(
        root.path(),
        vec![
            "replace",
            "form.hwpx",
            "--find",
            "검토",
            "--replace-with",
            "확정",
            "--node-id",
            "s0:t0",
        ],
        "",
    );
    assert_eq!(replacement["patch"]["operations"][0]["nodeId"], *native);
    let mut patch = preview["patch"].clone();
    patch["operations"][0]["nodeId"] = json!("s0:t0");
    std::fs::write(root.path().join("patch.json"), patch.to_string()).unwrap();
    let planned = cli(
        root.path(),
        vec!["plan", "form.hwpx", "--patch", "patch.json"],
        "",
    );
    assert_eq!(planned["changes"], preview["changes"]);
    assert_eq!(
        mcp(
            root.path(),
            "plan",
            json!({"path":"form.hwpx","patch":patch})
        ),
        planned
    );
    cli(
        root.path(),
        vec![
            "apply",
            "form.hwpx",
            "--patch",
            "patch.json",
            "-o",
            "new.hwpx",
            "--acknowledge-risks",
        ],
        "",
    );
}
#[test]
fn test_alias_duplicate_and_stale_text_are_refused_atomically() {
    let root = workspace();
    let original = std::fs::read(root.path().join("form.hwpx")).unwrap();
    let preview = hwp_workbench::service::invoke(
        root.path(),
        "set",
        &json!({"path":"form.hwpx","nodeId":"s0:t0","text":"new"}),
    )
    .unwrap();
    let mut patch = preview["patch"].clone();
    let mut duplicate = patch["operations"][0].clone();
    duplicate["nodeId"] = json!("s0:t0");
    patch["operations"]
        .as_array_mut()
        .unwrap()
        .push(duplicate.clone());
    assert_eq!(
        hwp_workbench::service::invoke(
            root.path(),
            "plan",
            &json!({"path":"form.hwpx","patch":patch})
        )
        .unwrap_err()
        .code,
        "BAD_PATCH"
    );
    duplicate["expectedText"] = json!("wrong");
    patch["operations"] = json!([duplicate]);
    assert_eq!(
        hwp_workbench::service::invoke(
            root.path(),
            "plan",
            &json!({"path":"form.hwpx","patch":patch})
        )
        .unwrap_err()
        .code,
        "STALE_TEXT"
    );
    assert_eq!(
        hwp_workbench::service::invoke(
            root.path(),
            "set",
            &json!({"path":"form.hwpx","nodeId":"s9:t999","text":"new"})
        )
        .unwrap_err()
        .code,
        "NODE_NOT_FOUND"
    );
    assert_eq!(
        std::fs::read(root.path().join("form.hwpx")).unwrap(),
        original
    );
}
#[test]
fn test_read_text_view_and_json_default() {
    let root = workspace();
    let expected =
        hwp_workbench::service::invoke(root.path(), "read", &json!({"path":"form.hwpx","limit":1}))
            .unwrap();
    let mut output = Vec::new();
    let mut errors = Vec::new();
    assert_eq!(
        cli::run(
            [
                "hwp-workbench",
                "read",
                "form.hwpx",
                "--root",
                root.path().to_str().unwrap(),
                "--limit",
                "1",
                "--format",
                "text"
            ],
            std::io::empty(),
            &mut output,
            &mut errors
        ),
        0
    );
    assert_eq!(
        String::from_utf8(output).unwrap(),
        format!(
            "{}\nsha256={} nextOffset=1\n",
            expected["text"].as_str().unwrap(),
            expected["sha256"].as_str().unwrap()
        )
    );
    assert_eq!(
        cli(root.path(), vec!["read", "form.hwpx", "--limit", "1"], ""),
        expected
    );
}
#[test]
fn test_search_positional_query_and_legacy_flag() {
    let root = workspace();
    assert_eq!(
        cli(root.path(), vec!["search", "form.hwpx", "검토"], ""),
        cli(
            root.path(),
            vec!["search", "form.hwpx", "--query", "검토"],
            ""
        )
    );
    let mut errors = Vec::new();
    assert_eq!(
        cli::run(
            [
                "hwp-workbench",
                "search",
                "form.hwpx",
                "검토",
                "--query",
                "other",
                "--root",
                root.path().to_str().unwrap()
            ],
            std::io::empty(),
            Vec::new(),
            &mut errors
        ),
        2
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&errors).unwrap()["error"]["code"],
        "BAD_ARGUMENTS"
    );
}
#[test]
fn test_fill_repeated_fields_and_json() {
    let root = workspace();
    std::fs::write(
        root.path().join("fields.json"),
        json!({"기관명":"New=Org","담당자":"Jane"}).to_string(),
    )
    .unwrap();
    assert_eq!(
        cli(
            root.path(),
            vec![
                "fill",
                "form.hwpx",
                "--field",
                "기관명=New=Org",
                "--field",
                "담당자=Jane"
            ],
            ""
        ),
        cli(
            root.path(),
            vec!["fill", "form.hwpx", "--json", "fields.json"],
            ""
        )
    );
    for flags in [
        vec![],
        vec!["--field", "broken"],
        vec!["--field", "=value"],
        vec!["--field", "기관명=A", "--field", "기관명=B"],
    ] {
        let mut argv = vec![
            "hwp-workbench",
            "fill",
            "form.hwpx",
            "--root",
            root.path().to_str().unwrap(),
        ];
        argv.extend(flags);
        let mut errors = Vec::new();
        assert_eq!(cli::run(argv, std::io::empty(), Vec::new(), &mut errors), 2);
        assert_eq!(
            serde_json::from_slice::<Value>(&errors).unwrap()["error"]["code"],
            "BAD_ARGUMENTS"
        );
    }
}
#[test]
fn test_every_subcommand_and_argument_has_help() {
    let mut parser = cli::parser();
    parser.build();
    for command in parser
        .get_subcommands()
        .filter(|command| !command.is_hide_set())
    {
        assert!(command.get_about().is_some(), "{}", command.get_name());
        for argument in command.get_arguments() {
            assert!(
                argument.get_help().is_some(),
                "{} {}",
                command.get_name(),
                argument.get_id()
            );
        }
    }
    let expected: Value = serde_json::from_str(include_str!("../src/cli_help.json")).unwrap();
    let mut output = Vec::new();
    assert_eq!(
        cli::run(
            ["hwp-workbench", "--help"],
            std::io::empty(),
            &mut output,
            Vec::new()
        ),
        0
    );
    assert_eq!(
        String::from_utf8(output).unwrap(),
        expected["root"].as_str().unwrap()
    );
}
#[test]
fn test_skill_contract_and_three_recipes() {
    let root = workspace();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/hwp-workbench/SKILL.md");
    let skill = std::fs::read_to_string(&source).unwrap();
    assert!(skill.lines().count() < 150);
    assert!(skill.starts_with("---\nname: hwp-workbench\ndescription:"));
    for term in [
        "HWP",
        "HWPX",
        "한글 문서",
        "HWP Workbench",
        "{{HWP_WORKBENCH}}",
        "native controls",
        "untrusted data",
        "never as instructions",
        "warnings",
        "references/patch-format.md",
    ] {
        assert!(skill.contains(term), "{term}");
    }
    let reference =
        std::fs::read_to_string(source.parent().unwrap().join("references/patch-format.md"))
            .unwrap();
    for term in [
        "schemaVersion",
        "baseSha256",
        "set_text",
        "nodeId",
        "expectedText",
        "STALE_DOCUMENT",
        "STALE_TEXT",
        "UNSUPPORTED_ELEMENT",
        "OUTPUT_EXISTS",
    ] {
        assert!(reference.contains(term));
    }
    cli(root.path(), vec!["read", "form.hwpx"], "");
    cli(root.path(), vec!["search", "form.hwpx", "기관명"], "");
    for (index, flags) in [
        vec![
            "fill",
            "form.hwpx",
            "--field",
            "기관명=New Org",
            "--field",
            "담당자=Jane",
        ],
        vec![
            "replace",
            "form.hwpx",
            "--find",
            "검토",
            "--replace-with",
            "확정",
        ],
        vec![
            "set",
            "form.hwpx",
            "--node-id",
            "s0:t0",
            "--text",
            "New title",
        ],
    ]
    .into_iter()
    .enumerate()
    {
        let preview = cli(root.path(), flags, "");
        assert!(!preview["changes"].as_array().unwrap().is_empty());
        let patch = format!("review-{index}.patch.json");
        std::fs::write(root.path().join(&patch), preview["patch"].to_string()).unwrap();
        cli(
            root.path(),
            vec!["plan", "form.hwpx", "--patch", &patch],
            "",
        );
        cli(
            root.path(),
            vec![
                "propose",
                "form.hwpx",
                "--patch",
                &patch,
                "--title",
                "Please review",
                "--author",
                "codex",
            ],
            "",
        );
        let output = format!("edited-{index}.hwpx");
        cli(
            root.path(),
            vec![
                "apply",
                "form.hwpx",
                "--patch",
                &patch,
                "-o",
                &output,
                "--acknowledge-risks",
            ],
            "",
        );
    }
    assert_eq!(
        cli(root.path(), vec!["session", "form.hwpx"], "")["proposals"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}
#[test]
fn proposal_envelope_transport_parity() {
    let root = workspace();
    let preview = cli(
        root.path(),
        vec![
            "set",
            "form.hwpx",
            "--node-id",
            "s0:t0",
            "--text",
            "Reviewed",
        ],
        "",
    );
    std::fs::write(root.path().join("preview.json"), preview.to_string()).unwrap();
    let result = cli(
        root.path(),
        vec![
            "propose",
            "form.hwpx",
            "--patch",
            "preview.json",
            "--title",
            "CLI review",
            "--author",
            "codex",
        ],
        "",
    );
    let record: Value = serde_json::from_slice(
        &std::fs::read(root.path().join(result["proposal"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    assert_eq!(record["author"], "codex");
    let result = mcp(
        root.path(),
        "propose",
        json!({"path":"form.hwpx","patch":preview,"title":"MCP review","notes":"Check the layout."}),
    );
    let record: Value = serde_json::from_slice(
        &std::fs::read(root.path().join(result["proposal"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    assert_eq!(record["notes"], "Check the layout.");
}
#[test]
fn absent_session_transport_parity() {
    let root = workspace();
    let expected =
        hwp_workbench::service::invoke(root.path(), "session", &json!({"path":"form.hwpx"}))
            .unwrap();
    assert_eq!(cli(root.path(), vec!["session", "form.hwpx"], ""), expected);
    assert_eq!(
        mcp(root.path(), "session", json!({"path":"form.hwpx"})),
        expected
    );
    assert!(!root.path().join(".hwp-workbench").exists());
}
#[test]
fn worker_failure_returns_two_with_one_worker_response() {
    let mut output = Vec::new();
    let mut errors = Vec::new();
    assert_eq!(
        cli::run(
            ["hwp-workbench", "__rhwp-worker"],
            std::io::empty(),
            &mut output,
            &mut errors
        ),
        2
    );
    assert!(errors.is_empty());
    assert_eq!(String::from_utf8_lossy(&output).lines().count(), 1);
    assert!(serde_json::from_slice::<Value>(&output).unwrap()["error"].is_object());
}
#[test]
fn rpc_non_string_method_matches_service_validation() {
    let root = workspace();
    let mut errors = Vec::new();
    assert_eq!(
        cli::run(
            [
                "hwp-workbench",
                "rpc",
                "--root",
                root.path().to_str().unwrap()
            ],
            json!({"method":1}).to_string().as_bytes(),
            Vec::new(),
            &mut errors
        ),
        2
    );
    let error: Value = serde_json::from_slice(&errors).unwrap();
    assert_eq!(error["error"]["code"], "BAD_ARGUMENTS");
    assert_eq!(error["error"]["message"], "method must be a string.");
}
#[test]
fn version_prints_only_the_oracle_version() {
    let mut output = Vec::new();
    assert_eq!(
        cli::run(
            ["hwp-workbench", "--version"],
            std::io::empty(),
            &mut output,
            Vec::new()
        ),
        0
    );
    assert_eq!(output, b"0.1.0-alpha.1\n");
}

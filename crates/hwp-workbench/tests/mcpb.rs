use hwp_workbench::mcpb;
use std::fs;
use std::io::Read;
use std::path::Path;

#[test]
fn desktop_config_uses_a_versioned_bundle_in_the_app_data_override() {
    let scratch = tempfile::tempdir().unwrap();
    let documents = scratch.path().join("documents");
    fs::create_dir(&documents).unwrap();
    let data_home = scratch.path().join("Application Support/workbench");
    let binary = env!("CARGO_BIN_EXE_hwp-workbench");
    let version = std::process::Command::new(binary)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(version.status.success());
    let version = String::from_utf8(version.stdout).unwrap();
    let result = std::process::Command::new(binary)
        .args(["mcp-config", "--client", "claude-desktop", "--root"])
        .arg(&documents)
        .env("HWP_WORKBENCH_DATA_HOME", &data_home)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(
        String::from_utf8(result.stdout).unwrap().trim(),
        data_home
            .join(format!("hwp-workbench-{}.mcpb", version.trim()))
            .to_str()
            .unwrap()
    );
    assert!(!data_home.exists(), "Printing setup must not write files");
}

#[cfg(target_os = "macos")]
#[test]
fn default_bundle_path_is_outside_checkout_and_signed_app_resources() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().join("home");
    let checkout = scratch.path().join("checkout");
    let contents = scratch.path().join("HWP Workbench.app/Contents");
    let resources = contents.join("Resources/app");
    for directory in [&home, &checkout, &resources] {
        fs::create_dir_all(directory).unwrap();
    }
    for source_root in [&checkout, &resources] {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_hwp-workbench"))
            .args(["mcp-config", "--client", "claude-desktop", "--root"])
            .arg(source_root)
            .current_dir(source_root)
            .env("HOME", &home)
            .env_remove("HWP_WORKBENCH_DATA_HOME")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(result.status.success(), "{:?}", result.stderr);
        let path = std::path::PathBuf::from(String::from_utf8(result.stdout).unwrap().trim());
        assert_eq!(
            path,
            mcpb::bundle_path_in(
                &home
                    .join("Library/Application Support")
                    .join(mcpb::APP_IDENTIFIER)
            )
        );
        assert!(!path.starts_with(source_root));
        assert!(!path.starts_with(contents.parent().unwrap()));
        assert!(!path.exists());
    }
}

#[test]
fn app_identifier_and_version_match_tauri_and_workspace() {
    let configuration: serde_json::Value =
        serde_json::from_str(include_str!("../../../app/src-tauri/tauri.conf.json")).unwrap();
    assert_eq!(configuration["identifier"], mcpb::APP_IDENTIFIER);
    let app_version = configuration["version"].as_str().unwrap();
    let server_version = hwp_workbench::service::VERSION;
    assert_eq!(app_version, server_version);
    assert_eq!(server_version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn test_build_archive_manifest_no_overwrite_and_packaged_startup() {
    let scratch = tempfile::tempdir().unwrap();
    let binary = scratch.path().join("hwp-workbench");
    fs::copy(env!("CARGO_BIN_EXE_hwp-workbench"), &binary).unwrap();
    let output = scratch.path().join("bundle.mcpb");
    mcpb::build(&binary, &output).unwrap();
    let original = fs::read(&output).unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    assert_eq!(archive.len(), 3);
    let manifest: serde_json::Value =
        serde_json::from_reader(archive.by_name("manifest.json").unwrap()).unwrap();
    assert_eq!(manifest["server"]["type"], "binary");
    assert_eq!(manifest["server"]["entry_point"], "server/hwp-workbench");
    assert_eq!(
        manifest["server"]["mcp_config"],
        serde_json::json!({"command":"${__dirname}/server/hwp-workbench", "args":["mcp", "--root", "${user_config.workspace_directory}"]})
    );
    assert_eq!(
        manifest["compatibility"],
        serde_json::json!({"platforms":["darwin"]})
    );
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        manifest["user_config"]["workspace_directory"]["type"],
        "directory"
    );
    assert_eq!(
        manifest["user_config"]["workspace_directory"]["required"],
        true
    );
    assert!(manifest["user_config"]["workspace_directory"]
        .get("default")
        .is_none());
    assert_eq!(manifest["icon"], "icon.png");
    let mut member = archive.by_name("server/hwp-workbench").unwrap();
    assert_eq!(member.unix_mode().unwrap() & 0o777, 0o755);
    let mut bytes = Vec::new();
    member.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, fs::read(&binary).unwrap());
    drop(member);
    assert!(archive.by_name("icon.png").unwrap().size() > 0);
    let tools: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/mcp_tools.json"))
            .unwrap(),
    )
    .unwrap();
    let listed = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| serde_json::json!({"name":tool["name"],"description":tool["description"]}))
        .collect::<Vec<_>>();
    assert_eq!(manifest["tools"], serde_json::json!(listed));
    let installed = scratch.path().join("installed");
    archive.extract(&installed).unwrap();
    // Generic extractors can ignore ZIP Unix mode; install as the manifest requires.
    use std::os::unix::fs::PermissionsExt;
    let server = installed.join("server/hwp-workbench");
    fs::set_permissions(&server, fs::Permissions::from_mode(0o755)).unwrap();
    let documents = scratch.path().join("documents");
    fs::create_dir(&documents).unwrap();
    let messages = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n";
    let mut child = std::process::Command::new(&server)
        .arg("mcp")
        .arg("--root")
        .arg(&documents)
        .current_dir(&documents)
        .env("PATH", "")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(messages).unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let responses = String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses.last().unwrap()["result"]["tools"], tools);
    assert!(mcpb::build(&binary, &output).is_err());
    assert_eq!(fs::read(&output).unwrap(), original);
}

#[test]
fn test_builder_excludes_non_source_files_and_includes_optional_icon() {
    let scratch = tempfile::tempdir().unwrap();
    let binary = scratch.path().join("hwp-workbench");
    fs::write(&binary, b"binary bytes").unwrap();
    fs::write(
        scratch.path().join("credentials.json"),
        b"private credentials",
    )
    .unwrap();
    fs::write(scratch.path().join("user.hwpx"), b"private document").unwrap();
    let output = scratch.path().join("bundle.mcpb");
    mcpb::build(&binary, &output).unwrap();
    let archive = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
    let members: std::collections::HashSet<_> = archive.file_names().collect();
    assert_eq!(
        members,
        ["manifest.json", "icon.png", "server/hwp-workbench"]
            .into_iter()
            .collect()
    );
}

#[test]
fn unreadable_or_symlinked_binary_does_not_create_output() {
    let scratch = tempfile::tempdir().unwrap();
    let output = scratch.path().join("absent/bundle.mcpb");
    assert!(mcpb::build(&scratch.path().join("missing"), &output).is_err());
    assert!(!output.parent().unwrap().exists());
    let binary = scratch.path().join("binary");
    fs::write(&binary, b"source").unwrap();
    let link = scratch.path().join("linked");
    std::os::unix::fs::symlink(&binary, &link).unwrap();
    assert!(mcpb::build(&link, &output).is_err());
    assert!(!output.exists());
    let destination = scratch.path().join("existing");
    fs::write(&destination, b"keep").unwrap();
    std::os::unix::fs::symlink(&destination, output.parent().unwrap()).unwrap();
    assert!(mcpb::build(&binary, &output).is_err());
    assert_eq!(fs::read(destination).unwrap(), b"keep");
}

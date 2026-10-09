use hwp_core::{
    document::{sha, Document},
    workspace::Workspace,
};
use hwp_workbench::agent_link;
use hwp_workbench::agent_ops;
use serde_json::{json, Value};
use std::{fs, path::Path};
struct Case {
    _temp: tempfile::TempDir,
    workspace: Workspace,
}
impl Case {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("form.hwpx"),
            include_bytes!("../../../fixtures/generated/review-form.hwpx"),
        )
        .unwrap();
        let workspace = Workspace::new(temp.path()).unwrap();
        Self {
            _temp: temp,
            workspace,
        }
    }
    fn root(&self) -> &Path {
        self.workspace.root()
    }
    fn doc(&self) -> Document {
        Document::new(self.workspace.read("form.hwpx").unwrap(), "form.hwpx").unwrap()
    }
    fn preview(&self) -> Value {
        agent_ops::replace(
            &self.doc(),
            &json!("검토"),
            &json!("확정"),
            &json!(false),
            &json!(true),
            None,
            &json!(1000),
        )
        .unwrap()
    }
    fn propose(&self) -> Value {
        agent_link::propose(
            &self.workspace,
            "form.hwpx",
            &mut self.doc(),
            &json!({"patch":self.preview(),"title":"Check text"}),
        )
        .unwrap()
    }
    fn report(&self) -> Value {
        agent_link::session(&self.workspace, "form.hwpx", &self.doc()).unwrap()
    }
    fn outcome(&self, p: &Value, status: &str) -> (std::path::PathBuf, Value) {
        let path = self.root().join(p["proposal"].as_str().unwrap());
        let bytes = fs::read(&path).unwrap();
        let record: Value = serde_json::from_slice(&bytes).unwrap();
        let ids: Vec<Value> = record["patch"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["nodeId"].clone())
            .collect();
        let (accepted, declined) = match status {
            "accepted" => (ids.clone(), vec![]),
            "declined" => (vec![], ids.clone()),
            _ => (ids[..1].to_vec(), ids[1..].to_vec()),
        };
        (
            path.with_extension("outcome.json"),
            json!({"schemaVersion":1,"status":status,"decidedAt":"now","proposalSha256":sha(&bytes),"acceptedNodeIds":accepted,"declinedNodeIds":declined}),
        )
    }
}
#[test]
fn proposal_envelope_and_full_patch_cli_mcp() {
    let c = Case::new();
    let preview = c.preview();
    let r = c.propose();
    assert_eq!(r["changes"], preview["changes"]);
    let p: Value =
        serde_json::from_slice(&fs::read(c.root().join(r["proposal"].as_str().unwrap())).unwrap())
            .unwrap();
    assert_eq!(p["kind"], "hwp-workbench-proposal");
    assert_eq!(p["patch"], preview["patch"]);
    assert_eq!(p["author"], "agent");
}
#[test]
fn complete_validation_before_creating_directories() {
    let c = Case::new();
    for (key, value, code) in [
        ("expectedText", json!("wrong"), "STALE_TEXT"),
        ("op", json!("delete"), "BAD_PATCH"),
        ("text", json!("bad\ntext"), "CONTROL_TEXT"),
        ("nodeId", json!("missing"), "NODE_NOT_FOUND"),
    ] {
        let mut p = c.preview()["patch"].clone();
        p["operations"][0][key] = value;
        assert_eq!(
            agent_link::propose(
                &c.workspace,
                "form.hwpx",
                &mut c.doc(),
                &json!({"patch":p,"title":"Review"})
            )
            .unwrap_err()
            .code,
            code
        );
    }
    assert!(!c.root().join(".hwp-workbench").exists());
    let mut p = c.preview()["patch"].clone();
    p["baseSha256"] = json!("wrong");
    assert_eq!(
        agent_link::propose(
            &c.workspace,
            "form.hwpx",
            &mut c.doc(),
            &json!({"patch":p,"title":"Review"})
        )
        .unwrap_err()
        .code,
        "STALE_DOCUMENT"
    );
    for options in [
        json!({"title":""}),
        json!({"title":"\u{1c}"}),
        json!({"title":"Review","author":" "}),
        json!({"title":"Review","notes":null}),
    ] {
        let mut args = options;
        args["patch"] = c.preview();
        assert_eq!(
            agent_link::propose(&c.workspace, "form.hwpx", &mut c.doc(), &args)
                .unwrap_err()
                .code,
            "BAD_ARGUMENTS"
        );
    }
}
#[test]
fn confined_nested_path_and_symlink_directories() {
    use std::os::unix::fs::symlink;
    let c = Case::new();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), c.root().join(".hwp-workbench")).unwrap();
    assert_eq!(
        agent_link::propose(
            &c.workspace,
            "form.hwpx",
            &mut c.doc(),
            &json!({"patch":c.preview(),"title":"Review"})
        )
        .unwrap_err()
        .code,
        "UNSAFE_PATH"
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    assert_eq!(
        agent_link::propose(
            &c.workspace,
            "../form.hwpx",
            &mut c.doc(),
            &json!({"patch":c.preview(),"title":"Review"})
        )
        .unwrap_err()
        .code,
        "OUTSIDE_WORKSPACE"
    );
    fs::remove_file(c.root().join(".hwp-workbench")).unwrap();
    fs::create_dir(c.root().join("nested")).unwrap();
    fs::copy(
        c.root().join("form.hwpx"),
        c.root().join("nested/form.hwpx"),
    )
    .unwrap();
    let result = agent_link::propose(
        &c.workspace,
        c.root().join("nested/form.hwpx").to_str().unwrap(),
        &mut c.doc(),
        &json!({"patch":c.preview(),"title":"Review"}),
    )
    .unwrap();
    assert!(result["proposal"]
        .as_str()
        .unwrap()
        .starts_with("nested/.hwp-workbench/proposals/"));
    fs::create_dir(c.root().join(".hwp-workbench")).unwrap();
    symlink(outside.path(), c.root().join(".hwp-workbench/proposals")).unwrap();
    assert_eq!(
        agent_link::propose(
            &c.workspace,
            "form.hwpx",
            &mut c.doc(),
            &json!({"patch":c.preview(),"title":"Review"})
        )
        .unwrap_err()
        .code,
        "UNSAFE_PATH"
    );
}
#[test]
fn proposal_limits_and_short_reference_normalization() {
    let c = Case::new();
    let mut p = agent_ops::set_text(&c.doc(), &json!("s0:t0"), &json!("new")).unwrap();
    p["patch"]["operations"][0]["nodeId"] = json!("s0:t0");
    let r = agent_link::propose(
        &c.workspace,
        "form.hwpx",
        &mut c.doc(),
        &json!({"patch":p,"title":"Review"}),
    )
    .unwrap();
    let record: Value =
        serde_json::from_slice(&fs::read(c.root().join(r["proposal"].as_str().unwrap())).unwrap())
            .unwrap();
    assert_eq!(
        record["patch"]["operations"][0]["nodeId"],
        c.doc().nodes[0].id
    );
    assert_eq!(
        agent_link::propose(
            &c.workspace,
            "form.hwpx",
            &mut c.doc(),
            &json!({"patch":c.preview(),"title":"Review","notes":"x".repeat(2*1024*1024)})
        )
        .unwrap_err()
        .code,
        "LIMIT"
    );
    let mut patch = c.preview()["patch"].clone();
    let first = patch["operations"][0].clone();
    patch["operations"] = json!(vec![first; 1001]);
    assert_eq!(
        agent_link::propose(
            &c.workspace,
            "form.hwpx",
            &mut c.doc(),
            &json!({"patch":patch,"title":"Review"})
        )
        .unwrap_err()
        .code,
        "BAD_PATCH"
    );
}
#[test]
fn absent_session_is_read_only_cli_mcp_parity() {
    let c = Case::new();
    assert_eq!(
        c.report(),
        json!({"linked":false,"session":null,"proposals":[]})
    );
    assert!(!c.root().join(".hwp-workbench").exists());
}
#[test]
fn desktop_session_and_per_document_isolation() {
    let c = Case::new();
    c.propose();
    let d = c.doc();
    let session = json!({"schemaVersion":1,"app":"hwp-workbench","appVersion":"0.1.0-alpha.1","document":{"name":d.name,"format":d.format,"sha256":d.hash},"selection":{"nodeId":d.nodes[0].id,"text":d.nodes[0].text},"staged":null,"updatedAt":"now"});
    fs::write(
        c.root().join(".hwp-workbench/form.hwpx.session.json"),
        serde_json::to_vec(&session).unwrap(),
    )
    .unwrap();
    assert_eq!(c.report()["session"], session);
    assert_eq!(c.report()["proposals"][0]["status"], "pending");
    let mut invalid = session;
    invalid["document"]["sha256"] = json!("0".repeat(64));
    fs::write(
        c.root().join(".hwp-workbench/form.hwpx.session.json"),
        serde_json::to_vec(&invalid).unwrap(),
    )
    .unwrap();
    assert_eq!(c.report()["linked"], false);
    assert_eq!(c.report()["session"], Value::Null);
    let other = Document::new(
        include_bytes!("../../../fixtures/generated/legacy-review.hwp"),
        "legacy.hwp",
    )
    .unwrap();
    assert_eq!(
        agent_link::session(&c.workspace, "legacy.hwp", &other).unwrap(),
        json!({"linked":false,"session":null,"proposals":[]})
    );
}
#[test]
fn outcome_binds_exact_proposal_bytes_and_node_partition() {
    let c = Case::new();
    let p = c.propose();
    let (path, valid) = c.outcome(&p, "accepted");
    let mut invalid = vec![valid.clone(); 6];
    invalid[0].as_object_mut().unwrap().remove("proposalSha256");
    invalid[1]["proposalSha256"] = json!("0".repeat(64));
    invalid[2]["acceptedNodeIds"] = json!(["unrelated"]);
    invalid[3]["acceptedNodeIds"].as_array_mut().unwrap().pop();
    let first = valid["acceptedNodeIds"][0].clone();
    invalid[4]["acceptedNodeIds"]
        .as_array_mut()
        .unwrap()
        .push(first.clone());
    invalid[5]["declinedNodeIds"] = json!([first]);
    for record in invalid {
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(c.report()["proposals"][0]["status"], "needsReview");
    }
    fs::write(&path, serde_json::to_vec(&valid).unwrap()).unwrap();
    assert_eq!(c.report()["proposals"][0]["status"], "accepted");
    let proposal = c.root().join(p["proposal"].as_str().unwrap());
    let mut bytes = fs::read(&proposal).unwrap();
    bytes.push(b' ');
    fs::write(&proposal, bytes).unwrap();
    assert_eq!(c.report()["proposals"][0]["status"], "needsReview");
}
#[test]
fn accepted_declined_partial_outcomes_and_unreadable() {
    let c = Case::new();
    let p = c.propose();
    for status in ["accepted", "declined", "partial"] {
        let (path, outcome) = c.outcome(&p, status);
        fs::write(path, serde_json::to_vec(&outcome).unwrap()).unwrap();
        assert_eq!(c.report()["proposals"][0]["status"], status);
    }
    let (path, _) = c.outcome(&p, "accepted");
    fs::write(path, "{bad").unwrap();
    assert_eq!(c.report()["proposals"][0]["status"], "needsReview");
}
#[test]
fn broken_oversized_symlink_and_nonregular_files_are_unreadable() {
    use std::os::unix::fs::symlink;
    let c = Case::new();
    let p = c.propose();
    let path = c.root().join(p["proposal"].as_str().unwrap());
    for data in ["{bad".into(), "[]".into(), "x".repeat(2 * 1024 * 1024 + 1)] {
        fs::write(&path, data).unwrap();
        assert_eq!(c.report()["proposals"][0]["status"], "unreadable");
    }
    fs::remove_file(&path).unwrap();
    symlink(c.root().join("form.hwpx"), &path).unwrap();
    assert_eq!(c.report()["proposals"][0]["status"], "unreadable");
    let session = c.root().join(".hwp-workbench/form.hwpx.session.json");
    for data in [
        "{bad".into(),
        "[]".into(),
        "null".into(),
        "{\"schemaVersion\":1}".into(),
        "[".repeat(1100),
    ] {
        fs::write(&session, data).unwrap();
        assert_eq!(c.report()["session"], Value::Null);
    }
    fs::remove_file(&path).unwrap();
    assert!(std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap()
        .success());
    assert_eq!(c.report()["proposals"][0]["status"], "unreadable");
}
#[test]
fn reads_at_most_200_proposals_with_per_file_size_limit() {
    let c = Case::new();
    c.propose();
    for i in 0..205 {
        fs::write(
            c.root().join(format!(
                ".hwp-workbench/proposals/form.hwpx.20261009T100000Z-{i:06x}.json"
            )),
            "{bad",
        )
        .unwrap();
    }
    assert_eq!(c.report()["proposals"].as_array().unwrap().len(), 200);
}
#[test]
fn document_name_prefix_does_not_include_another_document() {
    let c = Case::new();
    let p = c.propose();
    let path = c.root().join(p["proposal"].as_str().unwrap());
    let filename = path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .replace("form.hwpx.", "form.hwpx.other.hwpx.");
    fs::rename(&path, path.with_file_name(filename)).unwrap();
    assert_eq!(c.report()["proposals"], json!([]));
}
#[test]
fn link_directory_symlink_is_refused() {
    use std::os::unix::fs::symlink;
    let c = Case::new();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), c.root().join(".hwp-workbench")).unwrap();
    assert_eq!(
        agent_link::session(&c.workspace, "form.hwpx", &c.doc())
            .unwrap_err()
            .code,
        "UNSAFE_PATH"
    );
}
#[test]
fn null_outcome_requires_review() {
    let c = Case::new();
    let p = c.propose();
    let (path, _) = c.outcome(&p, "accepted");
    fs::write(path, "null").unwrap();
    assert_eq!(c.report()["proposals"][0]["status"], "needsReview");
}

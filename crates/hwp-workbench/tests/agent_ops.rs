use hwp_core::document::Document;
use hwp_workbench::agent_ops;
use serde_json::{json, Value};
fn doc() -> Document {
    Document::new(
        include_bytes!("../../../fixtures/generated/review-form.hwpx"),
        "form.hwpx",
    )
    .unwrap()
}
fn hwp() -> Document {
    Document::new(
        include_bytes!("../../../fixtures/generated/legacy-review.hwp"),
        "legacy.hwp",
    )
    .unwrap()
}
fn custom(section: &str) -> Document {
    let original = include_bytes!("../../../fixtures/generated/review-form.hwpx");
    let package = hwp_core::package_io::ZipPackage::new(original).unwrap();
    let bytes = package
        .rewrite(&std::collections::BTreeMap::from([(
            "Contents/section0.xml".into(),
            section.as_bytes().to_vec(),
        )]))
        .unwrap();
    Document::new(bytes, "form.hwpx").unwrap()
}
fn section(text: &str) -> Document {
    custom(&format!(
        "<hp:p xmlns:hp=\"{}\"><hp:run><hp:t>{text}</hp:t></hp:run></hp:p>",
        hwp_core::document::HP
    ))
}
fn cell(text: &str, extra: &str) -> String {
    format!("<hp:tc><hp:p><hp:run><hp:t>{text}</hp:t></hp:run></hp:p>{extra}</hp:tc>")
}
fn table(rows: &str) -> Document {
    custom(&format!(
        "<hp:tbl xmlns:hp=\"{}\">{rows}</hp:tbl>",
        hwp_core::document::HP
    ))
}
fn replace(d: &Document, find: &str, text: &str) -> Value {
    agent_ops::replace(
        d,
        &json!(find),
        &json!(text),
        &json!(false),
        &json!(true),
        None,
        &json!(1000),
    )
    .unwrap()
}
#[test]
fn read_paging_and_reference_mapping() {
    for d in [doc(), hwp()] {
        let r = agent_ops::read(&d, &json!(0), &json!(1)).unwrap();
        assert_eq!(r["sha256"], d.hash);
        assert_eq!(r["references"]["s0:t0"], d.nodes[0].id);
        assert!(r["text"].as_str().unwrap().contains("[s0:t0]"));
        assert_eq!(
            agent_ops::read(&d, &json!(d.nodes.len()), &json!(200)).unwrap()["text"],
            ""
        );
    }
}
#[test]
fn read_tables_and_read_only_nodes() {
    let mut d = doc();
    d.nodes[4].editable = false;
    assert!(agent_ops::read(&d, &json!(0), &json!(200)).unwrap()["text"]
        .as_str()
        .unwrap()
        .contains("| [s0:t3] 항목 / Item | [s0:t4] (read-only) 내용 / Details |"));
}
#[test]
fn search_offsets_case_context_and_limit() {
    for d in [doc(), hwp()] {
        let r =
            agent_ops::search(&d, &json!("검토"), &json!(false), &json!(false), &json!(1)).unwrap();
        let m = &r["matches"][0];
        let n = d.node(m["nodeId"].as_str().unwrap()).unwrap();
        assert_eq!(
            n.text
                .chars()
                .skip(m["start"].as_u64().unwrap() as usize)
                .take(2)
                .collect::<String>(),
            "검토"
        );
        assert_eq!(r["limitReached"], true);
    }
    assert_eq!(
        agent_ops::search(
            &doc(),
            &json!("internal"),
            &json!(false),
            &json!(true),
            &json!(100)
        )
        .unwrap()["matches"],
        json!([])
    );
    let d = doc();
    assert_eq!(
        agent_ops::search(
            &d,
            &json!("internal"),
            &json!(false),
            &json!(false),
            &json!(100)
        )
        .unwrap()["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        agent_ops::search(
            &d,
            &json!("J[oa]nathan"),
            &json!(true),
            &json!(false),
            &json!(100)
        )
        .unwrap()["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn regex_limits_and_invalid_arguments() {
    for q in [
        "(a+)+$",
        "a|aa",
        r"(a)\1",
        "a*",
        "a{1,100}",
        &"a".repeat(257),
        "[",
    ] {
        assert!(
            agent_ops::search(&doc(), &json!(q), &json!(true), &json!(false), &json!(100)).is_err()
        );
    }
    for v in [json!(0), json!(true)] {
        assert!(agent_ops::read(&doc(), &json!(0), &v).is_err());
    }
    assert!(agent_ops::read(&doc(), &json!(-1), &json!(200)).is_err());
}
#[test]
fn replace_patch_is_pure_and_allows_hwp_length_change() {
    for mut d in [doc(), hwp()] {
        let r = replace(&d, "검토", "확정됨");
        assert!(!r["changes"].as_array().unwrap().is_empty());
        assert!(d.edits.is_empty());
        d.patch(&r["patch"]).unwrap();
        let reopened = Document::new(d.serialize(true).unwrap(), "file").unwrap();
        assert!(reopened.nodes[0].text.contains("확정됨"));
    }
}
#[test]
fn replace_read_only_scope_and_occurrence_limit() {
    let mut d = doc();
    d.nodes[0].editable = false;
    assert_eq!(
        replace(&d, "검토", "확정")["skipped"][0]["code"],
        "UNSUPPORTED_ELEMENT"
    );
    let d = section("aaa");
    assert_eq!(
        agent_ops::replace(
            &d,
            &json!("a"),
            &json!("b"),
            &json!(false),
            &json!(true),
            Some(&json!(["s0:t0"])),
            &json!(2)
        )
        .unwrap()["changes"][0]["after"],
        "bba"
    );
    let d = doc();
    let node = d
        .nodes
        .iter()
        .find(|n| n.editable && n.text.contains("검토"))
        .unwrap();
    let scoped = agent_ops::replace(
        &d,
        &json!("검토"),
        &json!("확정"),
        &json!(false),
        &json!(true),
        Some(&json!([node.id])),
        &json!(1),
    )
    .unwrap();
    assert_eq!(scoped["changes"].as_array().unwrap().len(), 1);
    assert_eq!(scoped["changes"][0]["nodeId"], node.id);
    assert_eq!(
        agent_ops::replace(
            &d,
            &json!("검토"),
            &json!("확정"),
            &json!(false),
            &json!(true),
            Some(&json!(["missing"])),
            &json!(1)
        )
        .unwrap_err()
        .code,
        "NODE_NOT_FOUND"
    );
}
#[test]
fn fill_normalises_fixture_labels_and_reports_hwp() {
    use unicode_normalization::UnicodeNormalization;
    let label = "기 관 명 :".nfd().collect::<String>();
    let r = agent_ops::fill(
        &doc(),
        &json!({label:"새 기관","담당자 / Contact":"Jane","없음":"x"}),
    )
    .unwrap();
    assert_eq!(r["filled"].as_array().unwrap().len(), 2);
    assert_eq!(r["unresolved"], json!(["없음"]));
    assert_eq!(
        agent_ops::fill(&hwp(), &json!({"기관명":"x"})).unwrap()["unresolvedDetails"][0]["code"],
        "HWP_TABLE_MAPPING_UNAVAILABLE"
    );
    let mut d = doc();
    let result = agent_ops::fill(&d, &json!({"기관명":"새 기관"})).unwrap();
    assert!(d.edits.is_empty());
    d.patch(&result["patch"]).unwrap();
    assert!(Document::new(d.serialize(true).unwrap(), "file")
        .unwrap()
        .nodes
        .iter()
        .any(|n| n.text == "새 기관"));
}
#[test]
fn fill_below_ambiguous_and_multi_run_cells() {
    let d = table(&format!(
        "<hp:tr>{}</hp:tr><hp:tr>{}</hp:tr>",
        cell("Label:", ""),
        cell("", "")
    ));
    assert_eq!(
        agent_ops::fill(&d, &json!({"Label":"Value"})).unwrap()["filled"][0]["after"],
        "Value"
    );
    let d = table(&format!(
        "<hp:tr>{}{}</hp:tr><hp:tr>{}{}</hp:tr>",
        cell("Label", ""),
        cell("one", ""),
        cell("Label", ""),
        cell("two", "")
    ));
    assert_eq!(
        agent_ops::fill(&d, &json!({"Label":"Value"})).unwrap()["unresolvedDetails"][0]["code"],
        "AMBIGUOUS_LABEL"
    );
    let d=table(&format!("<hp:tr>{}<hp:tc><hp:p><hp:run><hp:t>a</hp:t><hp:t>b</hp:t></hp:run></hp:p></hp:tc></hp:tr>",cell("Label","")));
    let result = agent_ops::fill(&d, &json!({"Label":"Value"})).unwrap();
    assert_eq!(result["filled"], json!([]));
    assert_eq!(result["unresolved"], json!(["Label"]));
}
#[test]
fn set_text_patch_and_native_validation() {
    for mut d in [doc(), hwp()] {
        let r = agent_ops::set_text(&d, &json!("s0:t0"), &json!("short")).unwrap();
        assert_eq!(r["patch"]["operations"][0]["expectedText"], d.nodes[0].text);
        d.patch(&r["patch"]).unwrap();
        assert_eq!(
            Document::new(d.serialize(true).unwrap(), "file")
                .unwrap()
                .nodes[0]
                .text,
            "short"
        );
    }
    let d = hwp();
    assert_eq!(
        agent_ops::set_text(&d, &json!(doc().nodes[0].id), &json!("short"))
            .unwrap_err()
            .code,
        "NODE_NOT_FOUND"
    );
    assert!(d.edits.is_empty());
    assert_eq!(
        agent_ops::set_text(&d, &Value::Null, &json!("short"))
            .unwrap_err()
            .message,
        "Unknown node ID: None"
    );
    assert_eq!(
        agent_ops::set_text(&d, &json!([]), &json!("short"))
            .unwrap_err()
            .code,
        "REQUEST_FAILED"
    );
}
#[test]
fn replace_surrogate_pairs_and_literal_regex_replacement() {
    let mut d = hwp();
    let r = replace(&d, "검토", "😀");
    d.patch(&r["patch"]).unwrap();
    assert!(Document::new(d.serialize(true).unwrap(), "file")
        .unwrap()
        .nodes[0]
        .text
        .contains('😀'));
    assert_eq!(
        agent_ops::replace(
            &doc(),
            &json!("J[oa]nathan"),
            &json!(r"\1"),
            &json!(true),
            &json!(true),
            None,
            &json!(1000)
        )
        .unwrap()["changes"][0]["after"],
        r"\1 Jie"
    );
    let mut d = hwp();
    let r = replace(&d, "검", "😀");
    assert_eq!(r["skipped"], json!([]));
    d.patch(&r["patch"]).unwrap();
    assert!(Document::new(d.serialize(true).unwrap(), "file")
        .unwrap()
        .nodes[0]
        .text
        .contains('😀'));
}
#[test]
fn search_regex_match_cap_and_zero_width_offsets() {
    let d = section(&"a".repeat(1500));
    let r =
        agent_ops::search(&d, &json!("[a]"), &json!(true), &json!(false), &json!(1000)).unwrap();
    assert_eq!(r["matches"].as_array().unwrap().len(), 1000);
    assert_eq!(r["matches"][999]["start"], 999);
    assert_eq!(
        agent_ops::search(&d, &json!("^"), &json!(true), &json!(false), &json!(100)).unwrap()
            ["matches"][0]["end"],
        0
    );
}
#[test]
fn fill_merged_rows_nested_tables_and_protected_values() {
    let d = table(&format!(
        "<hp:tr>{}{}</hp:tr><hp:tr>{}</hp:tr>",
        cell("Opaque", "<hp:cellSpan colSpan=\"1\" rowSpan=\"2\"/>"),
        cell("Label:", ""),
        cell("", "")
    ));
    assert_eq!(
        agent_ops::fill(&d, &json!({"Label":"Value"})).unwrap()["filled"][0]["before"],
        ""
    );
    let mut d = doc();
    d.block_save = true;
    assert_eq!(
        agent_ops::fill(&d, &json!({"기관명":"new"})).unwrap()["filled"],
        json!([])
    );
    let inner = format!(
        "<hp:tbl><hp:tr>{}{}</hp:tr></hp:tbl>",
        cell("Inner", ""),
        cell("old", "")
    );
    let d = table(&format!(
        "<hp:tr>{}<hp:tc>{inner}</hp:tc></hp:tr>",
        cell("Outer", "")
    ));
    let result = agent_ops::fill(&d, &json!({"Inner":"new","Outer":"bad"})).unwrap();
    assert_eq!(result["filled"].as_array().unwrap().len(), 1);
    assert_eq!(result["filled"][0]["label"], "Inner");
    assert_eq!(result["unresolved"], json!(["Outer"]));
}
#[test]
fn read_keeps_multiple_runs_in_one_grid_cell() {
    let d=table("<hp:tr><hp:tc><hp:p><hp:run><hp:t>first</hp:t><hp:t>second</hp:t></hp:run></hp:p></hp:tc><hp:tc><hp:p><hp:run><hp:t>third</hp:t></hp:run></hp:p></hp:tc></hp:tr>");
    assert_eq!(
        agent_ops::read(&d, &json!(0), &json!(200)).unwrap()["text"],
        "| [s0:t0] first [s0:t1] second | [s0:t2] third |"
    );
}
#[test]
fn fill_uses_native_sparse_cell_addresses() {
    let d = table(&format!(
        "<hp:tr>{}</hp:tr><hp:tr>{}</hp:tr>",
        cell("Label", "<hp:cellAddr rowAddr=\"0\" colAddr=\"1\"/>"),
        cell("Unrelated", "<hp:cellAddr rowAddr=\"1\" colAddr=\"0\"/>")
    ));
    assert_eq!(
        agent_ops::fill(&d, &json!({"Label":"Value"})).unwrap()["unresolved"],
        json!(["Label"])
    );
    let d = table(&format!(
        "<hp:tr>{}{}</hp:tr>",
        cell("Value", "<hp:cellAddr rowAddr=\"0\" colAddr=\"2\"/>"),
        cell("Label", "<hp:cellAddr rowAddr=\"0\" colAddr=\"1\"/>")
    ));
    assert_eq!(
        agent_ops::fill(&d, &json!({"Label":"New"})).unwrap()["filled"][0]["before"],
        "Value"
    );
}
#[test]
fn fill_refuses_invalid_native_cell_addresses() {
    let d = table(&format!(
        "<hp:tr>{}{}</hp:tr>",
        cell("Label", "<hp:cellAddr rowAddr=\"0\" colAddr=\"invalid\"/>"),
        cell("old", "")
    ));
    assert_eq!(
        agent_ops::fill(&d, &json!({"Label":"New"})).unwrap()["unresolvedDetails"][0]["code"],
        "TABLE_MAPPING_UNAVAILABLE"
    );
}
#[test]
fn search_context_is_short_for_long_literal_matches() {
    let d = section(&format!("before {} after", "x".repeat(500)));
    let r = agent_ops::search(
        &d,
        &json!("x".repeat(500)),
        &json!(false),
        &json!(false),
        &json!(100),
    )
    .unwrap();
    assert_eq!(r["matches"][0]["start"], 7);
    assert_eq!(r["matches"][0]["end"], 507);
    assert!(r["matches"][0]["context"].as_str().unwrap().chars().count() <= 120);
}
#[test]
fn fill_uses_native_span_instead_of_clamped_preview_span() {
    let d=table(&format!("<hp:tr>{}</hp:tr><hp:tr>{}</hp:tr>",cell("Label","<hp:cellAddr rowAddr=\"0\" colAddr=\"0\"/><hp:cellSpan rowSpan=\"101\" colSpan=\"1\"/>"),cell("Value","<hp:cellAddr rowAddr=\"101\" colAddr=\"0\"/>")));
    assert_eq!(
        agent_ops::fill(&d, &json!({"Label":"New"})).unwrap()["filled"][0]["before"],
        "Value"
    );
}
#[test]
fn python_regex_escapes_unicode_classes_and_boundaries() {
    let d = section("a&#x345; ! Iİıi é 한글");
    for (query, count) in [
        (r"\ba\b", 1),
        (r"\w", 8),
        (r"\!", 1),
        (r"\A", 1),
        (r"\Z", 1),
        (r"\b한글\b", 1),
    ] {
        let r =
            agent_ops::search(&d, &json!(query), &json!(true), &json!(true), &json!(100)).unwrap();
        assert_eq!(r["matches"].as_array().unwrap().len(), count, "{query}");
    }
    let r = agent_ops::search(&d, &json!("i"), &json!(false), &json!(false), &json!(100)).unwrap();
    assert_eq!(r["matches"].as_array().unwrap().len(), 4);
    assert_eq!(
        agent_ops::search(&d, &json!(r"\q"), &json!(true), &json!(true), &json!(100))
            .unwrap_err()
            .message,
        r"bad escape \q at position 0"
    );
}
#[test]
fn python_regex_permitted_escapes_and_exact_errors() {
    let d = section("😀 ! [] abc Iİıi ſK");
    for (query, count) in [
        (r"\U0001F600", 1),
        (r"\x21", 1),
        (r"\u0021", 1),
        (r"\!", 1),
        (r"[]]", 1),
        (r"[[]", 1),
        (r"[a-z]", 9),
    ] {
        let r =
            agent_ops::search(&d, &json!(query), &json!(true), &json!(false), &json!(100)).unwrap();
        assert_eq!(r["matches"].as_array().unwrap().len(), count, "{query}");
    }
    for (query, message) in [
        (r"\pL", r"bad escape \p at position 0"),
        ("[z-a]", "bad character range z-a at position 1"),
        ("[]", "unterminated character set at position 0"),
    ] {
        assert_eq!(
            agent_ops::search(&d, &json!(query), &json!(true), &json!(true), &json!(100))
                .unwrap_err()
                .message,
            message
        );
    }
    for query in [r"\u0049", r"\U00000049", r"\İ", r"[\u0049-J]"] {
        assert_eq!(
            agent_ops::search(&d, &json!(query), &json!(true), &json!(false), &json!(100)).unwrap()
                ["matches"]
                .as_array()
                .unwrap()
                .len(),
            4,
            "{query}"
        );
    }
}
#[test]
fn python_regex_anchors_control_atoms_and_digit_restrictions() {
    let mut d = section("a");
    d.nodes[0].text = "a\n".into();
    let r = agent_ops::search(&d, &json!("$"), &json!(true), &json!(true), &json!(100)).unwrap();
    assert_eq!(r["matches"][0]["start"], 1);
    assert_eq!(r["matches"][0]["end"], 1);
    assert_eq!(r["matches"][1]["start"], 2);
    d.nodes[0].text = "\x07\x08\x0b".into();
    for query in [r"\a", r"[\b]", r"\v"] {
        assert_eq!(
            agent_ops::search(&d, &json!(query), &json!(true), &json!(true), &json!(100)).unwrap()
                ["matches"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    d.nodes[0].text.clear();
    assert_eq!(
        agent_ops::search(&d, &json!(r"\B"), &json!(true), &json!(true), &json!(100)).unwrap()
            ["matches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        agent_ops::search(
            &d,
            &json!(r"\uD800"),
            &json!(true),
            &json!(true),
            &json!(100)
        )
        .unwrap()["matches"],
        json!([])
    );
    for query in [r"\²", r"\፩"] {
        assert_eq!(
            agent_ops::search(&d, &json!(query), &json!(true), &json!(true), &json!(100))
                .unwrap_err()
                .code,
            "REGEX_UNSUPPORTED"
        );
    }
    assert!(agent_ops::search(&d, &json!(r"\Ⅷ"), &json!(true), &json!(true), &json!(100)).is_ok());
}
#[test]
fn recovery_hints_for_stable_codes() {
    use hwp_core::errors::WorkbenchError;
    for (code, action) in [
        ("STALE_DOCUMENT", "read"),
        ("STALE_TEXT", "read"),
        ("HWP_LENGTH_CHANGE_BLOCKED", "UTF-16"),
        ("OUTPUT_EXISTS", "filename"),
    ] {
        let error = WorkbenchError::new(code, "test").as_json();
        assert_eq!(error["code"], code);
        assert!(error["hint"].as_str().unwrap().contains(action));
        assert_eq!(error["retryable"], true);
    }
    assert_eq!(
        WorkbenchError::new("VERIFY_FAILED", "test").as_json()["retryable"],
        false
    );
}

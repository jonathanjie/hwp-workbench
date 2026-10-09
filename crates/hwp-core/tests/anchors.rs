mod common;
use common::*;
use serde_json::json;

#[test]
fn test_hwpx_runs_and_cell_share_top_level_owner() {
    let bytes = purpose_fixtures(
        "test_anchors.AnchorTests.test_hwpx_runs_and_cell_share_top_level_owner",
        "fixtures.make_hwpx",
    )
    .remove(0);
    let nodes = document(&bytes).nodes;
    assert_eq!(
        nodes
            .iter()
            .map(|node| node.anchor.clone())
            .collect::<Vec<_>>(),
        vec![
            json!({"section": 0, "paragraph": 0, "charOffset": 0, "inCell": false}),
            json!({"section": 0, "paragraph": 0, "charOffset": 2, "inCell": false}),
            json!({"section": 0, "paragraph": 1, "charOffset": null, "inCell": true}),
            json!({"section": 0, "paragraph": 1, "charOffset": 0, "inCell": false}),
            json!({"section": 0, "paragraph": 2, "charOffset": 0, "inCell": false}),
        ]
    );
}
#[test]
fn test_hwp_control_offsets_count_scalars() {
    let bytes = named_fixture("fixtures-cfb_from_streams-a8eceb273b5a9b71.hwp");
    assert_eq!(
        document(&bytes)
            .nodes
            .iter()
            .map(|node| node.anchor["charOffset"].clone())
            .collect::<Vec<_>>(),
        vec![json!(0), json!(3), json!(5)]
    );
    assert_body_ranges(&bytes);
}
#[test]
fn test_hwp_nested_cell_does_not_increment_body_index() {
    let bytes = named_fixture("fixtures-cfb_from_streams-9c8bbd6d55c59063.hwp");
    let nodes = document(&bytes).nodes;
    assert_eq!(
        nodes
            .iter()
            .map(|node| node.anchor["paragraph"].clone())
            .collect::<Vec<_>>(),
        vec![json!(0), json!(0), json!(1)]
    );
    assert_eq!(nodes[1].anchor["charOffset"], json!(null));
    assert_eq!(nodes[1].anchor["inCell"], true);
    assert_eq!(nodes[1].role, "table-cell");
}
#[test]
fn test_every_fixture_body_anchor_matches_rhwp_text_range() {
    let mut paths: Vec<_> = std::fs::read_dir(root().join("fixtures/generated"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("hwp" | "hwpx")
            )
        })
        .collect();
    let public: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("fixtures/public-manifest.json")).unwrap(),
    )
    .unwrap();
    for item in public["fixtures"].as_array().unwrap() {
        let path = root()
            .join("fixtures/public")
            .join(item["name"].as_str().unwrap());
        if !path.is_file() {
            eprintln!("SKIPPED: Public fixture not downloaded: {}", path.display());
            continue;
        }
        paths.push(path);
    }
    paths.sort();
    assert!(!paths.is_empty());
    for path in paths {
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            document(&bytes)
                .nodes
                .iter()
                .all(|node| node.anchor.is_object()),
            "Missing anchor: {}",
            path.display()
        );
        assert_body_ranges(&bytes);
    }
}
#[test]
fn test_hwpx_inline_controls_and_runs_match_direct_ranges() {
    let bytes = purpose_fixtures(
        "test_anchors.AnchorTests.test_hwpx_inline_controls_and_runs_match_direct_ranges",
        "fixtures.make_hwpx",
    )
    .remove(0);
    let document = document(&bytes);
    assert_eq!(document.nodes[1].anchor["charOffset"], 8);
    let native = rhwp::document_core::DocumentCore::from_bytes(&bytes).unwrap();
    let actual = native.get_text_range_native(0, 0, 0, usize::MAX).unwrap();
    assert_eq!(
        actual,
        document
            .nodes
            .iter()
            .map(|node| node.text.as_str())
            .collect::<String>()
    );
}
#[test]
fn test_paired_hwpx_controls_match_rhwp_offsets() {
    let bytes = purpose_fixtures(
        "test_anchors.AnchorTests.test_paired_hwpx_controls_match_rhwp_offsets",
        "fixtures.make_hwpx",
    )
    .remove(0);
    let document = document(&bytes);
    assert_eq!(document.nodes[1].anchor["charOffset"], 1);
    assert_eq!(document.nodes[1].text, "BC");
    assert_body_ranges(&bytes);
}
#[test]
fn test_hwp_table_caption_is_not_cell_text() {
    let bytes = named_fixture("fixtures-cfb_from_streams-b6681fbf5cf9a065.hwp");
    let nodes = document(&bytes).nodes;
    assert_eq!(nodes[0].anchor["inCell"], false);
    assert_eq!(nodes[1].anchor["inCell"], true);
    assert!(nodes[0].anchor["charOffset"].is_null());
}
#[test]
fn test_paired_auto_number_counts_inside_ctrl() {
    let bytes = purpose_fixtures(
        "test_anchors.AnchorTests.test_paired_auto_number_counts_inside_ctrl",
        "fixtures.make_hwpx",
    )
    .remove(0);
    assert_eq!(document(&bytes).nodes[0].anchor["charOffset"], 1);
    assert_body_ranges(&bytes);
}

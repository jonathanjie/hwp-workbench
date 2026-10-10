//! Frozen Python model evidence for all conversion-fixture inputs, not self-round-trips.
use hwp_convert::{read_docx, read_hwpx};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[test]
fn exported_fixtures_match_frozen_python_models_and_loss_counts() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("data/oracle.json")).unwrap();
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let payload = std::fs::read(directory.join(name)).unwrap();
        assert_eq!(
            hex::encode(Sha256::digest(&payload)),
            case["sha256"],
            "{name}: fixture provenance changed"
        );
        let result = if name.ends_with(".docx") {
            read_docx(&payload)
        } else {
            read_hwpx(&payload)
        };
        if let Some(expected) = case.get("error") {
            let error = result.unwrap_err();
            assert_eq!(error.code, expected["code"], "{name}");
            // Parser-library syntax messages vary; our own literal messages match.
            if error.code != "BAD_XML" {
                assert_eq!(error.message, expected["message"], "{name}");
            }
        } else {
            let result = serde_json::to_value(result.unwrap()).unwrap();
            assert_eq!(result, case["document"], "{name}: {}", case["purpose"]);
        }
    }
}

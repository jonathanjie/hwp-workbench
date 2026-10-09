use hwp_core::errors::{
    error_details, recovery, require, WorkbenchError, MAX_EXPANDED, MAX_FILE, MAX_NODES, MAX_PART,
};
use serde_json::json;

#[test]
fn error_json_matches_cli_and_mcp_detail() {
    let error = WorkbenchError::new("SOURCE_CHANGED", "changed");
    assert_eq!(
        error.as_json(),
        json!({"code":"SOURCE_CHANGED","message":"changed","hint":"Read or inspect the document again and rebuild the patch for its current SHA-256 before saving.","retryable":true})
    );
    assert_eq!(error_details(&error), error.as_json());
    assert_eq!(
        error_details(&std::io::Error::other("failure"))["code"],
        "REQUEST_FAILED"
    );
}

#[test]
fn limits_recovery_and_require_match_python() {
    assert_eq!(
        (MAX_FILE, MAX_EXPANDED, MAX_PART, MAX_NODES),
        (
            64 * 1024 * 1024,
            128 * 1024 * 1024,
            32 * 1024 * 1024,
            100_000
        )
    );
    assert!(require(true, "LIMIT", "large").is_ok());
    assert_eq!(
        require(false, "LIMIT", "large").unwrap_err().message,
        "large"
    );
    for code in ["BAD_CFB", "UNSAFE_XML", "ENCRYPTED", "unknown"] {
        assert!(!recovery(code).retryable);
    }
    assert!(recovery("BAD_ARGUMENTS").retryable);
}

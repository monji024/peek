// https://github.com/monji024

use std::path::PathBuf;
use std::process::Command;

#[test]
fn runs_comprehensive_bash_integration_suite() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = manifest_dir.join("tests/bash_integration_suite.py");
    let status = Command::new("python3")
        .arg(&script)
        .current_dir(&manifest_dir)
        .status()
        .expect("failed to execute python3 integration runner");
    assert!(status.success(), "Bash integration test suite failed");
}

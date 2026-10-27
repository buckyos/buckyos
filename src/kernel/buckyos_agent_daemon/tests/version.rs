use std::process::Command;

#[test]
fn cli_and_diagnostics_report_the_buckyos_runtime_version() {
    let binary = env!("CARGO_BIN_EXE_buckyos_agent_daemon");
    let expected = buckyos_kit::get_version();
    let output = Command::new(binary).arg("--version").output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("buckyos_agent_daemon {expected}")
    );

    let data = tempfile::tempdir().unwrap();
    for command in ["doctor", "status"] {
        let output = Command::new(binary)
            .arg("--data-dir")
            .arg(data.path())
            .args([command, "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["version"], expected);
    }
}

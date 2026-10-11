use std::process::Command;

#[test]
fn runs_script_and_exits_with_code() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("script.json");
    std::fs::write(
        &script,
        r#"[{"out":"one"},{"err":"warn"},{"out":"two"},{"exit":7}]"#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_fake-agent"))
        .arg(&script)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "warn");
}

#[test]
fn copies_stdin_to_file() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("stdin.txt");
    let script = dir.path().join("script.json");
    std::fs::write(&script, serde_json::json!([{"stdin_to": copy}]).to_string()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_fake-agent"))
        .arg(&script)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all("多行\n\"提示词\"".as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "多行\n\"提示词\"");
}

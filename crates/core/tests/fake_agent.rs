use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

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

#[test]
fn first_line_arrives_before_a_long_sleep() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("script.json");
    std::fs::write(&script, r#"[{"out":"x"},{"sleep_ms":5000}]"#).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_fake-agent"))
        .arg(&script)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Some(Ok(line)) = BufReader::new(stdout).lines().next() {
            let _ = tx.send(line);
        }
    });
    let first = rx.recv_timeout(Duration::from_secs(2));
    // Kill and reap before asserting, so a failed assertion cannot leave the child running.
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(first.expect("first stdout line within 2 s"), "x");
}

#[test]
fn repeat_out_keeps_printing() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("script.json");
    std::fs::write(&script, r#"[{"repeat_out":"t","every_ms":20}]"#).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_fake-agent"))
        .arg(&script)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut seen = 0;
    while seen < 2 {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) if line == "t" => seen += 1,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    // Kill and reap before asserting, so a failed assertion cannot leave the child running.
    let _ = child.kill();
    let _ = child.wait();
    assert!(seen >= 2, "saw {seen} line(s) equal to \"t\" within 2 s");
}

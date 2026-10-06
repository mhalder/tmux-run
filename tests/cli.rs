use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_suffix() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{pid}-{nanos}")
}

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn wait_reports_recorded_exit_status() {
    let state_dir = std::env::temp_dir().join(format!("tmux-run-cli-test-{}", unique_suffix()));
    std::fs::create_dir_all(&state_dir).expect("failed to create temp state dir");

    if !tmux_available() {
        eprintln!("skipping: tmux is not on PATH");
        let _ = std::fs::remove_dir_all(&state_dir);
        return;
    }

    let bin = env!("CARGO_BIN_EXE_tmux-run");

    let start = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .args(["cli-e2e", "--", "bash", "-c", "exit 7"])
        .output()
        .expect("failed to run tmux-run");

    assert!(
        start.status.success(),
        "tmux-run start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );

    let start_stdout = String::from_utf8_lossy(&start.stdout);
    let session = start_stdout
        .lines()
        .find_map(|line| line.strip_prefix("session: ").map(str::trim))
        .map(str::to_owned)
        .expect("tmux-run did not print a session line");

    let wait = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .args(["wait", &session, "--timeout", "30"])
        .output()
        .expect("failed to run tmux-run wait");

    let wait_stdout = String::from_utf8_lossy(&wait.stdout);
    let wait_stderr = String::from_utf8_lossy(&wait.stderr);
    assert_eq!(
        wait.status.code(),
        Some(7),
        "expected wait to exit with recorded status 7; stdout: {wait_stdout}; stderr: {wait_stderr}"
    );
    assert!(
        wait_stdout.contains("status: 7"),
        "expected `status: 7` in output: {wait_stdout}"
    );

    let _ = Command::new("tmux")
        .args(["kill-session", "-t", &session])
        .output();
    let _ = std::fs::remove_dir_all(&state_dir);
}

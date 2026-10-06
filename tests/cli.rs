use std::path::PathBuf;
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

/// Creates a unique temp directory for `XDG_STATE_HOME`, or returns `None` (and
/// removes the directory) when tmux is unavailable so the test is skipped.
fn prepare_state_dir() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("tmux-run-cli-test-{}", unique_suffix()));
    std::fs::create_dir_all(&dir).expect("failed to create temp state dir");
    if !tmux_available() {
        eprintln!("skipping: tmux is not on PATH");
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    Some(dir)
}

fn session_name_from(start_stdout: &[u8]) -> String {
    String::from_utf8_lossy(start_stdout)
        .lines()
        .find_map(|line| line.strip_prefix("session: ").map(str::trim))
        .map(str::to_owned)
        .expect("tmux-run did not print a session line")
}

#[test]
fn wait_reports_recorded_exit_status() {
    let Some(state_dir) = prepare_state_dir() else {
        return;
    };
    let bin = env!("CARGO_BIN_EXE_tmux-run");

    let start = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .env("TMUX_TMPDIR", &state_dir)
        .args(["cli-e2e", "--", "bash", "-c", "exit 7"])
        .output()
        .expect("failed to run tmux-run");

    assert!(
        start.status.success(),
        "tmux-run start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );

    let session = session_name_from(&start.stdout);

    let wait = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .env("TMUX_TMPDIR", &state_dir)
        .args(["wait", &session, "--timeout", "30"])
        .output()
        .expect("failed to run tmux-run wait");

    let wait_stdout = String::from_utf8_lossy(&wait.stdout);
    let wait_stderr = String::from_utf8_lossy(&wait.stderr);
    assert_eq!(
        wait.status.code(),
        Some(7),
        "expected wait to exit with recorded status 7; stdout: {wait_stdout}; stderr: {wait_stderr}; state: {}",
        state_dir.display()
    );
    assert!(
        wait_stdout.contains("status: 7"),
        "expected `status: 7` in output: {wait_stdout}"
    );

    let _ = Command::new("tmux")
        .env("TMUX_TMPDIR", &state_dir)
        .args(["kill-session", "-t", &session])
        .output();
    let _ = std::fs::remove_dir_all(&state_dir);
}

#[test]
fn wait_reports_status_when_output_lacks_trailing_newline() {
    let Some(state_dir) = prepare_state_dir() else {
        return;
    };
    let bin = env!("CARGO_BIN_EXE_tmux-run");

    let start = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .env("TMUX_TMPDIR", &state_dir)
        .args([
            "cli-e2e-nl",
            "--",
            "bash",
            "-c",
            "printf 'no newline'; exit 7",
        ])
        .output()
        .expect("failed to run tmux-run");

    assert!(
        start.status.success(),
        "tmux-run start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );

    let session = session_name_from(&start.stdout);

    let wait = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .env("TMUX_TMPDIR", &state_dir)
        .args(["wait", &session, "--timeout", "30"])
        .output()
        .expect("failed to run tmux-run wait");

    let wait_stdout = String::from_utf8_lossy(&wait.stdout);
    let wait_stderr = String::from_utf8_lossy(&wait.stderr);
    assert_eq!(
        wait.status.code(),
        Some(7),
        "expected wait to exit with recorded status 7; stdout: {wait_stdout}; stderr: {wait_stderr}; state: {}",
        state_dir.display()
    );
    assert!(
        wait_stdout.contains("status: 7"),
        "expected `status: 7` in output: {wait_stdout}"
    );

    let _ = Command::new("tmux")
        .env("TMUX_TMPDIR", &state_dir)
        .args(["kill-session", "-t", &session])
        .output();
    let _ = std::fs::remove_dir_all(&state_dir);
}

use std::path::PathBuf;
use std::process::Command;

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn wait_reports_recorded_exit_status() {
    if !tmux_available() {
        eprintln!("skipping: tmux is not on PATH");
        return;
    }

    let bin = env!("CARGO_BIN_EXE_tmux-run");

    let start = Command::new(bin)
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
    let log_path = start_stdout
        .lines()
        .find_map(|line| line.strip_prefix("log: ").map(str::trim))
        .map(PathBuf::from);

    let wait = Command::new(bin)
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
    if let Some(log) = log_path
        && let Some(dir) = log.parent()
    {
        let _ = std::fs::remove_dir_all(dir);
    }
}

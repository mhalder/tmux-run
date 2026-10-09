use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);

// Both end-to-end tests drive the same global tmux state, and cargo runs tests
// on parallel threads. Per-test `TMUX_TMPDIR` isolates the servers, and this
// lock serializes the tests so one test's session teardown never overlaps
// another's server startup; cargo's parallelism is not part of what they verify.
static TMUX_LOCK: Mutex<()> = Mutex::new(());

fn unique_suffix() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    // Clock granularity is not a uniqueness guarantee: macOS time has
    // microsecond resolution, so two threads in the same process can observe
    // the same pid and nanos. The counter makes each call unique regardless.
    let counter = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{pid}-{nanos}-{counter}")
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

/// Creates a short-lived tmux socket directory under `/tmp`. tmux appends
/// `/tmux-<uid>/default` to this path, and a unix socket path is limited to
/// 104 bytes on macOS. `std::env::temp_dir()` is `$TMPDIR` there — a long
/// `/var/folders` path — so a socket directory derived from it can overflow
/// that limit; the short, fixed `/tmp` prefix keeps the socket reachable.
fn prepare_socket_dir() -> PathBuf {
    let dir = PathBuf::from(format!("/tmp/tmux-run-sock-{}", unique_suffix()));
    std::fs::create_dir_all(&dir).expect("failed to create tmux socket dir");
    dir
}

fn session_name_from(start_stdout: &[u8]) -> String {
    String::from_utf8_lossy(start_stdout)
        .lines()
        .find_map(|line| line.strip_prefix("session: ").map(str::trim))
        .map(str::to_owned)
        .expect("tmux-run did not print a session line")
}

/// A single diagnostic string appended to the `wait` assertion so a future
/// `Missing` failure explains which process produced what, rather than leaving
/// only the `wait` output to guess from.
fn wait_failure_diagnostics(
    start: &std::process::Output,
    state_dir: &Path,
    socket_dir: &Path,
) -> String {
    let mut parts = String::new();
    parts.push_str(&format!(
        "; start stdout: `{}`; start stderr: `{}`",
        String::from_utf8_lossy(&start.stdout).trim(),
        String::from_utf8_lossy(&start.stderr).trim()
    ));
    parts.push_str(&format!(
        "; state_dir exists: {} contents: {}",
        state_dir.exists(),
        dir_contents(state_dir)
    ));
    parts.push_str(&format!(
        "; socket_dir exists: {} contents: {}",
        socket_dir.exists(),
        dir_contents(socket_dir)
    ));
    let sessions = Command::new("tmux")
        .env("TMUX_TMPDIR", socket_dir)
        .args(["list-sessions"])
        .output();
    let sessions = match sessions {
        Ok(output) => format!(
            "status {}; stdout: `{}`; stderr: `{}`",
            output.status,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(err) => format!("error: {err}"),
    };
    parts.push_str(&format!("; tmux list-sessions: {sessions}"));
    parts
}

fn dir_contents(dir: &Path) -> String {
    if !dir.exists() {
        return "n/a".to_string();
    }
    let mut entries = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let read = match std::fs::read_dir(&path) {
            Ok(read) => read,
            Err(err) => {
                entries.push(format!("<unreadable {}: {err}>", path.display()));
                continue;
            }
        };
        for entry in read.flatten() {
            let path = entry.path();
            let kind = match entry.file_type() {
                Ok(file_type) if file_type.is_dir() => 'd',
                Ok(_) => 'f',
                Err(_) => '?',
            };
            entries.push(format!("{kind}:{}", path.display()));
            if entry
                .file_type()
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false)
            {
                stack.push(path);
            }
        }
    }
    if entries.is_empty() {
        "empty".to_string()
    } else {
        entries.join("; ")
    }
}

#[test]
fn wait_reports_recorded_exit_status() {
    let _tmux_guard = TMUX_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(state_dir) = prepare_state_dir() else {
        return;
    };
    let socket_dir = prepare_socket_dir();
    let bin = env!("CARGO_BIN_EXE_tmux-run");

    let start = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .env("TMUX_TMPDIR", &socket_dir)
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
        .env("TMUX_TMPDIR", &socket_dir)
        .args(["wait", &session, "--timeout", "30"])
        .output()
        .expect("failed to run tmux-run wait");

    let wait_stdout = String::from_utf8_lossy(&wait.stdout);
    let wait_stderr = String::from_utf8_lossy(&wait.stderr);
    assert_eq!(
        wait.status.code(),
        Some(7),
        "expected wait to exit with recorded status 7; stdout: {wait_stdout}; stderr: {wait_stderr}; state: {}{}",
        state_dir.display(),
        wait_failure_diagnostics(&start, &state_dir, &socket_dir)
    );
    assert!(
        wait_stdout.contains("status: 7"),
        "expected `status: 7` in output: {wait_stdout}"
    );

    let _ = Command::new("tmux")
        .env("TMUX_TMPDIR", &socket_dir)
        .args(["kill-session", "-t", &session])
        .output();
    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&socket_dir);
}

#[test]
fn wait_reports_status_when_output_lacks_trailing_newline() {
    let _tmux_guard = TMUX_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(state_dir) = prepare_state_dir() else {
        return;
    };
    let socket_dir = prepare_socket_dir();
    let bin = env!("CARGO_BIN_EXE_tmux-run");

    let start = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .env("TMUX_TMPDIR", &socket_dir)
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
        .env("TMUX_TMPDIR", &socket_dir)
        .args(["wait", &session, "--timeout", "30"])
        .output()
        .expect("failed to run tmux-run wait");

    let wait_stdout = String::from_utf8_lossy(&wait.stdout);
    let wait_stderr = String::from_utf8_lossy(&wait.stderr);
    assert_eq!(
        wait.status.code(),
        Some(7),
        "expected wait to exit with recorded status 7; stdout: {wait_stdout}; stderr: {wait_stderr}; state: {}{}",
        state_dir.display(),
        wait_failure_diagnostics(&start, &state_dir, &socket_dir)
    );
    assert!(
        wait_stdout.contains("status: 7"),
        "expected `status: 7` in output: {wait_stdout}"
    );

    let _ = Command::new("tmux")
        .env("TMUX_TMPDIR", &socket_dir)
        .args(["kill-session", "-t", &session])
        .output();
    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&socket_dir);
}

#[test]
fn list_and_show_report_a_finished_task() {
    let _tmux_guard = TMUX_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(state_dir) = prepare_state_dir() else {
        return;
    };
    let socket_dir = prepare_socket_dir();
    let bin = env!("CARGO_BIN_EXE_tmux-run");

    let start = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .env("TMUX_TMPDIR", &socket_dir)
        .args([
            "cli-e2e-ls",
            "--",
            "bash",
            "-c",
            "printf 'hello\\nworld\\n'; exit 3",
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
        .env("TMUX_TMPDIR", &socket_dir)
        .args(["wait", &session, "--timeout", "30"])
        .output()
        .expect("failed to run tmux-run wait");

    let wait_stdout = String::from_utf8_lossy(&wait.stdout);
    let wait_stderr = String::from_utf8_lossy(&wait.stderr);
    assert_eq!(
        wait.status.code(),
        Some(3),
        "expected wait to exit with recorded status 3; stdout: {wait_stdout}; stderr: {wait_stderr}; state: {}{}",
        state_dir.display(),
        wait_failure_diagnostics(&start, &state_dir, &socket_dir)
    );

    let list = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .env("TMUX_TMPDIR", &socket_dir)
        .arg("list")
        .output()
        .expect("failed to run tmux-run list");
    assert!(
        list.status.success(),
        "list failed: {}",
        String::from_utf8_lossy(&list.stderr)
    );
    let list_out = String::from_utf8_lossy(&list.stdout);
    assert!(
        list_out.contains(&format!("{session}\tdone 3\t")),
        "expected `{session}\\tdone 3\\t` in list output: {list_out}"
    );

    let list_json = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .env("TMUX_TMPDIR", &socket_dir)
        .args(["list", "--json"])
        .output()
        .expect("failed to run tmux-run list --json");
    assert!(list_json.status.success());
    let list_json_out = String::from_utf8_lossy(&list_json.stdout);
    assert!(
        list_json_out.contains(&format!("\"session\":\"{session}\"")),
        "expected session in JSON list output: {list_json_out}"
    );
    assert!(
        list_json_out.contains("\"state\":\"done\"") && list_json_out.contains("\"status\":3"),
        "expected done state with status 3 in JSON list output: {list_json_out}"
    );

    let show = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .env("TMUX_TMPDIR", &socket_dir)
        .args(["show", &session])
        .output()
        .expect("failed to run tmux-run show");
    assert!(
        show.status.success(),
        "show failed: {}",
        String::from_utf8_lossy(&show.stderr)
    );
    let show_out = String::from_utf8_lossy(&show.stdout);
    assert!(
        show_out.contains("status: done 3"),
        "expected `status: done 3` in show output: {show_out}"
    );
    assert!(show_out.contains("hello"), "show output: {show_out}");
    assert!(show_out.contains("world"), "show output: {show_out}");

    let _ = Command::new("tmux")
        .env("TMUX_TMPDIR", &socket_dir)
        .args(["kill-session", "-t", &session])
        .output();
    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&socket_dir);
}

#[test]
fn clean_and_rm_remove_finished_task_state() {
    let _tmux_guard = TMUX_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(state_dir) = prepare_state_dir() else {
        return;
    };
    let socket_dir = prepare_socket_dir();
    let bin = env!("CARGO_BIN_EXE_tmux-run");

    let run = |args: &[&str]| {
        Command::new(bin)
            .env("XDG_STATE_HOME", &state_dir)
            .env("TMUX_TMPDIR", &socket_dir)
            .args(args)
            .output()
            .expect("failed to run tmux-run")
    };
    let state_session_dir = |session: &str| state_dir.join("tmux-run").join(session);

    let start = run(&["cli-e2e-clean", "--", "bash", "-c", "exit 0"]);
    assert!(
        start.status.success(),
        "tmux-run start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    let session = session_name_from(&start.stdout);

    let wait = run(&["wait", &session, "--timeout", "30"]);
    assert_eq!(
        wait.status.code(),
        Some(0),
        "expected wait to exit 0; stderr: {}",
        String::from_utf8_lossy(&wait.stderr)
    );
    assert!(state_session_dir(&session).exists());

    let dry = run(&["clean", "--dry-run"]);
    assert!(dry.status.success());
    assert!(
        String::from_utf8_lossy(&dry.stdout).contains("would remove"),
        "expected dry-run to report what it would remove: {}",
        String::from_utf8_lossy(&dry.stdout)
    );
    assert!(state_session_dir(&session).exists());

    let clean = run(&["clean"]);
    assert!(clean.status.success());
    assert!(
        !state_session_dir(&session).exists(),
        "clean should have removed {}",
        state_session_dir(&session).display()
    );

    // A second task exercises the single-session form.
    let start = run(&["cli-e2e-rm", "--", "bash", "-c", "exit 0"]);
    assert!(start.status.success());
    let session = session_name_from(&start.stdout);
    assert_eq!(
        run(&["wait", &session, "--timeout", "30"]).status.code(),
        Some(0)
    );

    let rm = run(&["rm", &session]);
    assert!(
        rm.status.success(),
        "rm failed: {}",
        String::from_utf8_lossy(&rm.stderr)
    );
    assert!(!state_session_dir(&session).exists());

    // Removing a session whose state is gone is exit 3, not a crash.
    assert_eq!(run(&["rm", &session]).status.code(), Some(3));

    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&socket_dir);
}

#[test]
fn failed_start_leaves_no_state_behind() {
    let Some(state_dir) = prepare_state_dir() else {
        return;
    };
    let empty_bin = state_dir.join("empty-bin");
    std::fs::create_dir_all(&empty_bin).expect("failed to create empty bin dir");
    let bin = env!("CARGO_BIN_EXE_tmux-run");

    // A PATH without tmux makes `tmux new-session` fail after the state
    // directory is created but before any log exists, the window in which a
    // failed start used to leave state that list and clean could never reach.
    let start = Command::new(bin)
        .env("XDG_STATE_HOME", &state_dir)
        .env("PATH", &empty_bin)
        .args(["failed-start", "--", "bash", "-c", "exit 0"])
        .output()
        .expect("failed to run tmux-run");

    assert!(
        !start.status.success(),
        "start should fail without tmux on PATH: {}",
        String::from_utf8_lossy(&start.stderr)
    );

    let leftover = dir_contents(&state_dir.join("tmux-run"));
    assert!(
        !leftover.contains("failed-start"),
        "a failed start must not leave unreachable state: {leftover}"
    );

    let _ = std::fs::remove_dir_all(&state_dir);
}

#[test]
fn unique_suffix_is_unique_across_threads() {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};
    use std::thread;

    const THREADS: usize = 16;
    const PER_THREAD: usize = 64;

    let seen = Arc::new(Mutex::new(HashSet::new()));
    thread::scope(|scope| {
        for _ in 0..THREADS {
            let seen = Arc::clone(&seen);
            scope.spawn(move || {
                for _ in 0..PER_THREAD {
                    let suffix = unique_suffix();
                    let mut seen = seen.lock().unwrap();
                    assert!(seen.insert(suffix.clone()), "duplicate suffix: {suffix}");
                }
            });
        }
    });

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), THREADS * PER_THREAD);
    let sample: Vec<String> = seen.iter().take(3).cloned().collect();
    println!(
        "generated {} unique suffixes across {} threads (sample: {})",
        seen.len(),
        THREADS,
        sample.join(", ")
    );
}

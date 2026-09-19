use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DONE_MARKER_PREFIX: &str = "__DONE__:";
/// Only the log tail is scanned for the marker; it is appended last.
const LOG_TAIL_BYTES: u64 = 8 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const SESSION_CHECK_EVERY: u32 = 4;
const HELP: &str = "Usage: tmux-run <task-name> -- <command> [args...]\n       tmux-run wait <session-name> [--timeout <seconds>]\n\nStarts <command> in a detached tmux session and writes stdout/stderr to a log file.\n`tmux-run wait` blocks until that log records __DONE__:<status> and exits with that status.\n\nArguments:\n  <task-name>        Name used to build the tmux session name\n  --                 Separates tmux-run arguments from the command\n  <command> [args...] Command and arguments to run under bash\n\nWait arguments:\n  <session-name>     Session name printed by tmux-run\n  --timeout <secs>   Give up after this long and exit 124\n\nOutput:\n  session            tmux session name\n  log                path to combined stdout/stderr log\n  completion marker  __DONE__:<status> appended when the command exits\n\nWait exit status:\n  <status>           status recorded in the completion marker\n  124                --timeout elapsed\n  3                  session ended or never existed without a marker\n  2                  usage error\n\nExamples:\n  tmux-run build -- cargo test\n  tmux-run wait build_1234-5678 --timeout 600\n  tmux-run deploy -- bash -lc 'echo start; ./deploy.sh'\n";

#[derive(Debug, PartialEq, Eq)]
enum CliAction {
    Help,
    Run(Cli),
    Wait(WaitCli),
}

#[derive(Debug, PartialEq, Eq)]
struct Cli {
    task_name: String,
    command: Vec<OsString>,
}

#[derive(Debug, PartialEq, Eq)]
struct WaitCli {
    session_name: String,
    timeout: Option<Duration>,
}

#[derive(Debug)]
struct RuntimePaths {
    session_name: String,
    script_path: PathBuf,
    log_path: PathBuf,
}

fn main() {
    match run() {
        Ok(code) => process::exit(code),
        Err(err) => {
            eprintln!("error: {err}");
            eprintln!("usage: tmux-run <task-name> -- <command> [args...]");
            eprintln!("       tmux-run wait <session-name> [--timeout <seconds>]");
            process::exit(2);
        }
    }
}

fn run() -> Result<i32, String> {
    match parse_args(env::args_os().skip(1))? {
        CliAction::Help => {
            print!("{HELP}");
            Ok(0)
        }
        CliAction::Run(cli) => {
            start_task(&cli)?;
            Ok(0)
        }
        CliAction::Wait(wait) => run_wait(&wait),
    }
}

/// Block on an already-started task and report the status from its marker.
fn run_wait(wait: &WaitCli) -> Result<i32, String> {
    match wait_for_completion(&wait.session_name, wait.timeout)? {
        WaitOutcome::Completed(status) => {
            println!("session: {}", wait.session_name);
            println!("status: {status}");
            Ok(i32::from(status))
        }
        WaitOutcome::TimedOut => {
            eprintln!("error: timed out waiting for session {}", wait.session_name);
            Ok(124)
        }
        WaitOutcome::EndedWithoutMarker => {
            eprintln!(
                "error: session {} ended without writing a completion marker",
                wait.session_name
            );
            Ok(3)
        }
        WaitOutcome::Missing => {
            eprintln!("error: no running session or log for {}", wait.session_name);
            Ok(3)
        }
    }
}

fn start_task(cli: &Cli) -> Result<(), String> {
    let paths = build_runtime_paths(&cli.task_name)
        .map_err(|err| format!("failed to prepare paths: {err}"))?;
    let script = render_script(&cli.command);

    fs::write(&paths.script_path, script)
        .map_err(|err| format!("failed to write {}: {err}", paths.script_path.display()))?;

    let runner = format!(
        "bash {} >{} 2>&1",
        shell_quote_path(&paths.script_path),
        shell_quote_path(&paths.log_path)
    );

    let output = Command::new("tmux")
        .args(["new-session", "-d", "-s"])
        .arg(&paths.session_name)
        .arg(&runner)
        .output()
        .map_err(|err| format!("failed to start tmux: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "tmux new-session failed with status {}{}{}",
            output.status,
            if stderr.trim().is_empty() { "" } else { ": " },
            if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            }
        ));
    }

    println!("session: {}", paths.session_name);
    println!("log: {}", paths.log_path.display());
    println!("wait: tmux-run wait {}", shell_quote(&paths.session_name));
    println!("completion marker: {DONE_MARKER_PREFIX}<status>");
    println!(
        "attach: tmux attach -t {}",
        shell_quote(&paths.session_name)
    );
    println!("follow log: tail -f {}", shell_quote_path(&paths.log_path));

    Ok(())
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<CliAction, String> {
    let mut args = args.into_iter();
    let first = args
        .next()
        .ok_or_else(|| "missing task name".to_string())?
        .into_string()
        .map_err(|_| "task name must be valid UTF-8".to_string())?;

    if first == "--help" || first == "-h" {
        return Ok(CliAction::Help);
    }

    if first == "wait" {
        // `wait -- <command>` still names a task literally called "wait".
        return match args.next() {
            Some(separator) if separator == "--" => parse_command(first, args),
            Some(session) => {
                let session_name = session
                    .into_string()
                    .map_err(|_| "session name must be valid UTF-8".to_string())?;
                parse_wait(session_name, args)
            }
            None => Err("missing session name after `wait`".to_string()),
        };
    }

    parse_run(first, args)
}

fn parse_run(
    task_name: String,
    mut args: impl Iterator<Item = OsString>,
) -> Result<CliAction, String> {
    if task_name.is_empty() {
        return Err("task name must not be empty".to_string());
    }

    match args.next() {
        Some(separator) if separator == "--" => {}
        _ => return Err("missing `--` before command".to_string()),
    }

    parse_command(task_name, args)
}

fn parse_command(
    task_name: String,
    args: impl Iterator<Item = OsString>,
) -> Result<CliAction, String> {
    let command: Vec<OsString> = args.collect();
    if command.is_empty() {
        return Err("missing command after `--`".to_string());
    }

    Ok(CliAction::Run(Cli { task_name, command }))
}

fn parse_wait(
    session_name: String,
    mut args: impl Iterator<Item = OsString>,
) -> Result<CliAction, String> {
    if session_name == "--help" || session_name == "-h" {
        return Ok(CliAction::Help);
    }

    if !is_valid_session_name(&session_name) {
        return Err(format!("invalid session name: {session_name}"));
    }

    let mut timeout: Option<Duration> = None;
    while let Some(arg) = args.next() {
        let arg = arg
            .into_string()
            .map_err(|_| "arguments must be valid UTF-8".to_string())?;
        match arg.as_str() {
            "--timeout" => {
                let raw = args
                    .next()
                    .ok_or_else(|| "--timeout requires a value in seconds".to_string())?
                    .into_string()
                    .map_err(|_| "--timeout value must be valid UTF-8".to_string())?;
                let seconds: u64 = raw
                    .parse()
                    .map_err(|_| format!("invalid --timeout value: {raw}"))?;
                if seconds == 0 {
                    return Err("--timeout must be greater than zero".to_string());
                }
                timeout = Some(Duration::from_secs(seconds));
            }
            other => return Err(format!("unexpected argument for wait: {other}")),
        }
    }

    Ok(CliAction::Wait(WaitCli {
        session_name,
        timeout,
    }))
}

/// A session name is `normalize_task_name` output plus a pid and timestamp, so
/// anything outside this alphabet was not printed by tmux-run. Rejecting it
/// also keeps the name from escaping the state directory as a path.
fn is_valid_session_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn build_runtime_paths(task_name: &str) -> io::Result<RuntimePaths> {
    let unique = unique_suffix();
    let normalized = normalize_task_name(task_name);
    let session_name = format!("{normalized}_{unique}");
    let dir = state_root().join(&session_name);
    fs::create_dir_all(&dir)?;

    Ok(RuntimePaths {
        session_name,
        script_path: dir.join("run.sh"),
        log_path: dir.join("output.log"),
    })
}

/// Where task state lives: `XDG_STATE_HOME`, then `~/.local/state`, then the
/// system temp directory. Temp is the last resort so a temp sweep cannot take
/// the completion marker, and with it the exit status, before it is read.
fn state_root() -> PathBuf {
    state_root_from(env::var_os("XDG_STATE_HOME"), env::var_os("HOME"))
}

fn state_root_from(xdg_state_home: Option<OsString>, home: Option<OsString>) -> PathBuf {
    if let Some(dir) = xdg_state_home.filter(|value| !value.is_empty()) {
        return PathBuf::from(dir).join("tmux-run");
    }
    if let Some(home) = home.filter(|value| !value.is_empty()) {
        return PathBuf::from(home).join(".local/state/tmux-run");
    }
    env::temp_dir().join("tmux-run")
}

fn log_path_for(session_name: &str) -> PathBuf {
    state_root().join(session_name).join("output.log")
}

#[derive(Debug, PartialEq, Eq)]
enum WaitOutcome {
    Completed(u8),
    TimedOut,
    EndedWithoutMarker,
    Missing,
}

fn wait_for_completion(
    session_name: &str,
    timeout: Option<Duration>,
) -> Result<WaitOutcome, String> {
    poll_until_done(
        &log_path_for(session_name),
        || session_exists(session_name),
        timeout,
    )
}

fn poll_until_done<F>(
    log_path: &Path,
    mut session_alive: F,
    timeout: Option<Duration>,
) -> Result<WaitOutcome, String>
where
    F: FnMut() -> bool,
{
    let started = Instant::now();
    let mut tick: u32 = 0;

    loop {
        if let Some(status) = read_done_marker(log_path)? {
            return Ok(WaitOutcome::Completed(status));
        }

        if tick.is_multiple_of(SESSION_CHECK_EVERY) && !session_alive() {
            // The session is gone. Give a final write a moment to land before
            // concluding that no marker will ever arrive.
            thread::sleep(POLL_INTERVAL);
            if let Some(status) = read_done_marker(log_path)? {
                return Ok(WaitOutcome::Completed(status));
            }
            if !session_alive() {
                return Ok(if log_path.exists() {
                    WaitOutcome::EndedWithoutMarker
                } else {
                    WaitOutcome::Missing
                });
            }
        }

        if let Some(limit) = timeout
            && started.elapsed() >= limit
        {
            return Ok(WaitOutcome::TimedOut);
        }

        thread::sleep(POLL_INTERVAL);
        tick = tick.wrapping_add(1);
    }
}

/// `=` forces an exact session name, so an unrelated session whose name is a
/// prefix cannot answer for this one.
fn session_exists(session_name: &str) -> bool {
    Command::new("tmux")
        .args(["has-session", "-t"])
        .arg(format!("={session_name}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Read `__DONE__:<status>` from the last complete line of the log. Only the
/// tail is read: the marker is appended last and the log can be large.
fn read_done_marker(log_path: &Path) -> Result<Option<u8>, String> {
    let mut file = match fs::File::open(log_path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("failed to read {}: {err}", log_path.display())),
    };

    let len = file
        .metadata()
        .map_err(|err| format!("failed to stat {}: {err}", log_path.display()))?
        .len();
    if len > LOG_TAIL_BYTES {
        file.seek(SeekFrom::Start(len - LOG_TAIL_BYTES))
            .map_err(|err| format!("failed to seek {}: {err}", log_path.display()))?;
    }

    let mut tail = Vec::new();
    file.read_to_end(&mut tail)
        .map_err(|err| format!("failed to read {}: {err}", log_path.display()))?;

    // Ignore a trailing partial line: the marker is only trusted once its
    // newline has landed, which makes a torn read a retry rather than a wrong
    // status.
    let complete = tail
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    let text = String::from_utf8_lossy(&tail[..complete]);

    for line in text.lines().rev() {
        if let Some(rest) = line.strip_prefix(DONE_MARKER_PREFIX) {
            let rest = rest.trim();
            if rest.is_empty() {
                return Ok(None);
            }
            let status = rest
                .parse::<u8>()
                .map_err(|_| format!("malformed completion marker: {line}"))?;
            return Ok(Some(status));
        }
    }

    Ok(None)
}

fn unique_suffix() -> String {
    let pid = process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{pid}-{nanos}")
}

fn normalize_task_name(task_name: &str) -> String {
    let mut normalized = String::with_capacity(task_name.len());
    let mut previous_was_separator = false;

    for ch in task_name.chars() {
        let replacement = if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            ch
        } else {
            '_'
        };

        if replacement == '_' {
            if !previous_was_separator {
                normalized.push(replacement);
            }
            previous_was_separator = true;
        } else {
            normalized.push(replacement);
            previous_was_separator = false;
        }

        if normalized.len() >= 80 {
            break;
        }
    }

    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "task".to_string()
    } else {
        normalized.to_string()
    }
}

fn render_script(command: &[OsString]) -> String {
    let mut script = String::from("#!/usr/bin/env bash\nset -uo pipefail\ncmd=(");
    for arg in command {
        script.push(' ');
        script.push_str(&shell_quote_bytes(arg.as_os_str().as_bytes()));
    }
    script.push_str(
        " )\n\"${cmd[@]}\"\nstatus=$?\nprintf '__DONE__:%s\\n' \"$status\"\nexit \"$status\"\n",
    );
    script
}

fn shell_quote(value: &str) -> String {
    shell_quote_bytes(value.as_bytes())
}

fn shell_quote_path(path: &Path) -> String {
    shell_quote_bytes(path.as_os_str().as_bytes())
}

fn shell_quote_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "''".to_string();
    }

    let mut quoted = String::from("'");
    for &byte in bytes {
        if byte == b'\'' {
            quoted.push_str("'\\''");
        } else {
            let _ = quoted.write_char(byte as char);
        }
    }
    quoted.push('\'');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn parses_expected_cli_shape() {
        let CliAction::Run(cli) = parse_args(
            ["build", "--", "cargo", "test --all"]
                .into_iter()
                .map(OsString::from),
        )
        .unwrap() else {
            panic!("expected run action");
        };

        assert_eq!(cli.task_name, "build");
        assert_eq!(
            cli.command,
            vec![OsString::from("cargo"), OsString::from("test --all")]
        );
    }

    #[test]
    fn parses_help_flags() {
        assert_eq!(
            parse_args(["--help"].into_iter().map(OsString::from)).unwrap(),
            CliAction::Help
        );
        assert_eq!(
            parse_args(["-h"].into_iter().map(OsString::from)).unwrap(),
            CliAction::Help
        );
    }

    #[test]
    fn rejects_missing_separator_and_command() {
        assert!(parse_args(["build", "cargo"].into_iter().map(OsString::from)).is_err());
        assert!(parse_args(["build", "--"].into_iter().map(OsString::from)).is_err());
    }

    #[test]
    fn normalizes_task_name_for_tmux_session() {
        assert_eq!(normalize_task_name("deploy: prod/eu"), "deploy_prod_eu");
        assert_eq!(normalize_task_name("!!!"), "task");
    }

    #[test]
    fn renders_script_preserving_argument_boundaries() {
        let script = render_script(&[
            OsString::from("printf"),
            OsString::from("%s\\n"),
            OsString::from("hello world"),
            OsString::from("it's ok"),
        ]);

        assert!(script.contains("cmd=( 'printf' '%s\\n' 'hello world' 'it'\\''s ok' )"));
        assert!(script.contains("printf '__DONE__:%s\\n' \"$status\""));
    }

    #[test]
    fn generated_script_runs_command_and_appends_status_marker() {
        let dir = env::temp_dir().join(format!("tmux-run-test-{}", unique_suffix()));
        fs::create_dir_all(&dir).unwrap();
        let script_path = dir.join("run.sh");
        fs::write(
            &script_path,
            render_script(&[
                OsString::from("bash"),
                OsString::from("-c"),
                OsString::from("printf '<%s>\\n' \"$1\"; exit 7"),
                OsString::from("shim"),
                OsString::from("hello world"),
            ]),
        )
        .unwrap();

        let output = Command::new("bash").arg(&script_path).output().unwrap();

        assert_eq!(output.status.code(), Some(7));
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "<hello world>\n__DONE__:7\n"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_wait_subcommand() {
        let CliAction::Wait(wait) = parse_args(
            ["wait", "build_1-2", "--timeout", "30"]
                .into_iter()
                .map(OsString::from),
        )
        .unwrap() else {
            panic!("expected wait action");
        };

        assert_eq!(wait.session_name, "build_1-2");
        assert_eq!(wait.timeout, Some(Duration::from_secs(30)));
    }

    #[test]
    fn keeps_a_task_named_wait() {
        let CliAction::Run(cli) =
            parse_args(["wait", "--", "true"].into_iter().map(OsString::from)).unwrap()
        else {
            panic!("expected run action");
        };

        assert_eq!(cli.task_name, "wait");
    }

    #[test]
    fn rejects_invalid_wait_arguments() {
        assert!(parse_args(["wait"].into_iter().map(OsString::from)).is_err());
        assert!(parse_args(["wait", "../evil"].into_iter().map(OsString::from)).is_err());
        assert!(
            parse_args(
                ["wait", "s", "--timeout", "0"]
                    .into_iter()
                    .map(OsString::from)
            )
            .is_err()
        );
        assert!(
            parse_args(
                ["wait", "s", "--timeout", "soon"]
                    .into_iter()
                    .map(OsString::from)
            )
            .is_err()
        );
        assert!(parse_args(["wait", "s", "extra"].into_iter().map(OsString::from)).is_err());
        assert_eq!(
            parse_args(["wait", "--help"].into_iter().map(OsString::from)).unwrap(),
            CliAction::Help
        );
    }

    #[test]
    fn resolves_state_root_from_environment_values() {
        assert_eq!(
            state_root_from(
                Some(OsString::from("/state")),
                Some(OsString::from("/home/u"))
            ),
            PathBuf::from("/state/tmux-run")
        );
        assert_eq!(
            state_root_from(None, Some(OsString::from("/home/u"))),
            PathBuf::from("/home/u/.local/state/tmux-run")
        );
        assert_eq!(
            state_root_from(Some(OsString::new()), Some(OsString::from("/home/u"))),
            PathBuf::from("/home/u/.local/state/tmux-run")
        );
    }

    #[test]
    fn reads_done_marker_from_log_tail() {
        let dir = test_dir("marker");
        let log = dir.join("output.log");

        assert_eq!(read_done_marker(&log).unwrap(), None);
        fs::write(&log, "working\n").unwrap();
        assert_eq!(read_done_marker(&log).unwrap(), None);
        fs::write(&log, "working\n__DONE__:7\n").unwrap();
        assert_eq!(read_done_marker(&log).unwrap(), Some(7));
        fs::write(&log, "__DONE__:1\nmore\n__DONE__:2\n").unwrap();
        assert_eq!(read_done_marker(&log).unwrap(), Some(2));
        // A torn write is a retry, not a status.
        fs::write(&log, "__DONE__:1").unwrap();
        assert_eq!(read_done_marker(&log).unwrap(), None);
        fs::write(&log, "__DONE__:oops\n").unwrap();
        assert!(read_done_marker(&log).is_err());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn polls_until_marker_or_session_end() {
        let dir = test_dir("wait");
        let log = dir.join("output.log");

        assert_eq!(
            poll_until_done(&log, || false, None).unwrap(),
            WaitOutcome::Missing
        );

        fs::write(&log, "still running\n").unwrap();
        assert_eq!(
            poll_until_done(&log, || false, None).unwrap(),
            WaitOutcome::EndedWithoutMarker
        );

        assert_eq!(
            poll_until_done(&log, || true, Some(Duration::from_millis(200))).unwrap(),
            WaitOutcome::TimedOut
        );

        fs::write(&log, "done\n__DONE__:0\n").unwrap();
        assert_eq!(
            poll_until_done(&log, || true, None).unwrap(),
            WaitOutcome::Completed(0)
        );

        fs::remove_dir_all(dir).unwrap();
    }

    fn test_dir(label: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("tmux-run-test-{label}-{}", unique_suffix()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn quotes_empty_and_single_quote_bytes() {
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(
            shell_quote_path(Path::new(OsStr::new("/tmp/a b"))),
            "'/tmp/a b'"
        );
    }
}

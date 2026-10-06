use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DONE_MARKER_PREFIX: &str = "__DONE__:";
/// Only the log tail is scanned for the marker; it is appended last.
const LOG_TAIL_BYTES: u64 = 8 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const SESSION_CHECK_EVERY: u32 = 4;

const SKILL: &str = include_str!("../docs/SKILL.md");
const COMPLETION_BASH: &str = include_str!("../completions/tmux-run.bash");
const COMPLETION_ZSH: &str = include_str!("../completions/_tmux-run");
const COMPLETION_FISH: &str = include_str!("../completions/tmux-run.fish");

const HELP: &str = "Usage: tmux-run <task-name> -- <command> [args...]
       tmux-run wait <session-name> [--timeout <seconds>]
       tmux-run skill [--install [DIR]] [--check [DIR]]
       tmux-run completion <bash|zsh|fish> [--install] [--check]
       tmux-run --help | -h
       tmux-run --version | -V

Starts <command> in a detached tmux session and writes stdout/stderr to a log
file. `tmux-run wait` blocks until that log records __DONE__:<status> and exits
with that status.

Run arguments:
  <task-name>          Name used to build the tmux session name
  --                   Separates tmux-run arguments from the command
  <command> [args...]  Command and arguments to run under bash

Wait arguments:
  <session-name>       Session name printed by tmux-run
  --timeout <secs>     Give up after this long and exit 124

Skill subcommand:
  tmux-run skill                    Print the embedded agent skill to stdout
  tmux-run skill --install [DIR]    Write DIR/SKILL.md (default ~/.agents/skills/tmux-run)
  tmux-run skill --check [DIR]      Exit 1 if DIR/SKILL.md is missing or differs

Completion subcommand:
  tmux-run completion <shell>              Print the completion script for <shell>
  tmux-run completion <shell> --install    Install the completion script
  tmux-run completion <shell> --check      Exit 1 if the installed script is missing or differs
  zsh users: add the site-functions directory to $fpath

Exit status:
  <status>    status recorded in the completion marker (wait only)
  124         --timeout elapsed
  3           session ended or never existed without a marker
  2           usage error
  1           --check found the installed file missing or stale

Examples:
  tmux-run build -- cargo test
  tmux-run wait build_1234-5678 --timeout 600
  tmux-run deploy -- bash -lc 'echo start; ./deploy.sh'
  tmux-run skill --install
  tmux-run completion bash --install
";

#[derive(Debug, PartialEq, Eq)]
enum CliAction {
    Help,
    Version,
    Run(Cli),
    Wait(WaitCli),
    Skill(SkillCli),
    Completion(CompletionCli),
    CompleteSessions(Option<OsString>),
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

#[derive(Debug, PartialEq, Eq)]
struct SkillCli {
    mode: SkillMode,
    dir: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkillMode {
    Print,
    Install,
    Check,
}

#[derive(Debug, PartialEq, Eq)]
struct CompletionCli {
    shell: Shell,
    mode: CompletionMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    fn word(self) -> &'static str {
        match self {
            Shell::Bash => "bash",
            Shell::Zsh => "zsh",
            Shell::Fish => "fish",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompletionMode {
    Print,
    Install,
    Check,
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
        CliAction::Version => {
            println!("tmux-run {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        CliAction::Run(cli) => {
            start_task(&cli)?;
            Ok(0)
        }
        CliAction::Wait(wait) => run_wait(&wait),
        CliAction::Skill(skill) => run_skill(&skill),
        CliAction::Completion(completion) => run_completion(&completion),
        CliAction::CompleteSessions(prefix) => {
            for name in list_sessions(prefix.as_deref()) {
                println!("{name}");
            }
            Ok(0)
        }
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

    fs::write(&paths.script_path, &script)
        .map_err(|err| format!("failed to write {}: {err}", paths.script_path.display()))?;

    let mut runner = Vec::new();
    runner.extend_from_slice(b"bash ");
    runner.extend_from_slice(&quote_bytes(paths.script_path.as_os_str().as_bytes()));
    runner.extend_from_slice(b" >");
    runner.extend_from_slice(&quote_bytes(paths.log_path.as_os_str().as_bytes()));
    runner.extend_from_slice(b" 2>&1");
    let runner = OsString::from_vec(runner);

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
        .ok_or_else(|| "missing command".to_string())?
        .into_string()
        .map_err(|_| "arguments must be valid UTF-8".to_string())?;

    if first == "--help" || first == "-h" {
        return Ok(CliAction::Help);
    }
    if first == "--version" || first == "-V" {
        return Ok(CliAction::Version);
    }
    if first == "wait" {
        return parse_wait_subcommand(args);
    }
    if first == "skill" {
        return parse_skill(args);
    }
    if first == "completion" {
        return parse_completion(args);
    }
    if first == "__complete" {
        return parse_complete(args);
    }

    parse_run(first, args)
}

fn parse_wait_subcommand(mut args: impl Iterator<Item = OsString>) -> Result<CliAction, String> {
    // `wait -- <command>` still names a task literally called "wait".
    match args.next() {
        Some(separator) if separator == "--" => parse_command("wait".to_string(), args),
        Some(session) => {
            let session_name = session
                .into_string()
                .map_err(|_| "session name must be valid UTF-8".to_string())?;
            parse_wait(session_name, args)
        }
        None => Err("missing session name after `wait`".to_string()),
    }
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

fn parse_skill(mut args: impl Iterator<Item = OsString>) -> Result<CliAction, String> {
    let mut mode: Option<SkillMode> = None;
    let mut dir: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        let arg = arg
            .into_string()
            .map_err(|_| "arguments must be valid UTF-8".to_string())?;

        let wanted = match arg.as_str() {
            "--install" => SkillMode::Install,
            "--check" => SkillMode::Check,
            other => return Err(format!("unexpected argument for skill: {other}")),
        };

        if mode.is_some() {
            return Err("--install and --check are mutually exclusive".to_string());
        }
        mode = Some(wanted);

        // An optional directory may follow; a leading `-` means no directory.
        if let Some(next) = args.next() {
            let next = next
                .into_string()
                .map_err(|_| "arguments must be valid UTF-8".to_string())?;
            if next.starts_with('-') {
                return Err(format!("unexpected argument for skill: {next}"));
            }
            dir = Some(PathBuf::from(next));
        }
    }

    Ok(CliAction::Skill(SkillCli {
        mode: mode.unwrap_or(SkillMode::Print),
        dir,
    }))
}

fn parse_completion(mut args: impl Iterator<Item = OsString>) -> Result<CliAction, String> {
    let shell_arg = args
        .next()
        .ok_or_else(|| "completion requires a shell: bash, zsh, or fish".to_string())?
        .into_string()
        .map_err(|_| "shell must be valid UTF-8".to_string())?;

    let shell = match shell_arg.as_str() {
        "bash" => Shell::Bash,
        "zsh" => Shell::Zsh,
        "fish" => Shell::Fish,
        other => return Err(format!("unknown shell: {other}")),
    };

    let mut mode: Option<CompletionMode> = None;
    for arg in args {
        let arg = arg
            .into_string()
            .map_err(|_| "arguments must be valid UTF-8".to_string())?;
        let wanted = match arg.as_str() {
            "--install" => CompletionMode::Install,
            "--check" => CompletionMode::Check,
            other => return Err(format!("unexpected argument for completion: {other}")),
        };
        if mode.is_some() {
            return Err("--install and --check are mutually exclusive".to_string());
        }
        mode = Some(wanted);
    }

    Ok(CliAction::Completion(CompletionCli {
        shell,
        mode: mode.unwrap_or(CompletionMode::Print),
    }))
}

fn parse_complete(mut args: impl Iterator<Item = OsString>) -> Result<CliAction, String> {
    let what = args
        .next()
        .ok_or_else(|| "__complete requires a subcommand".to_string())?
        .into_string()
        .map_err(|_| "arguments must be valid UTF-8".to_string())?;

    if what != "sessions" {
        return Err(format!("unknown __complete subcommand: {what}"));
    }

    let prefix = args.next();
    if args.next().is_some() {
        return Err("too many arguments for __complete sessions".to_string());
    }

    Ok(CliAction::CompleteSessions(prefix))
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

/// Read `__DONE__:<status>` from the tail of the log. Only the tail is read:
/// the marker is appended last and the log can be large. The marker is accepted
/// either at the start of the last complete line (`__DONE__:0`), or concatenated
/// onto the end of it when the command's output had no trailing newline
/// (`no trailing newline__DONE__:0`). A trailing partial line is ignored so a
/// torn read is a retry rather than a wrong status.
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

    // The marker is appended last, so inspect the last complete line first: it
    // either starts with the marker, or the marker is concatenated onto the end
    // of output that did not end with a newline.
    if let Some(last_line) = text.lines().next_back() {
        let line = last_line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix(DONE_MARKER_PREFIX) {
            let rest = rest.trim();
            if rest.is_empty() {
                return Ok(None);
            }
            let status = rest
                .parse::<u8>()
                .map_err(|_| format!("malformed completion marker: {last_line}"))?;
            return Ok(Some(status));
        }
        if let Some(at) = line.rfind(DONE_MARKER_PREFIX) {
            let rest = line[at + DONE_MARKER_PREFIX.len()..].trim();
            if !rest.is_empty()
                && let Ok(status) = rest.parse::<u8>()
            {
                return Ok(Some(status));
            }
        }
    }

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

fn render_script(command: &[OsString]) -> Vec<u8> {
    let mut script = Vec::new();
    script.extend_from_slice(b"#!/usr/bin/env bash\nset -uo pipefail\ncmd=(");
    for arg in command {
        script.push(b' ');
        script.extend_from_slice(&quote_bytes(arg.as_os_str().as_bytes()));
    }
    script.extend_from_slice(
        b" )\n\"${cmd[@]}\"\nstatus=$?\nprintf '__DONE__:%s\\n' \"$status\"\nexit \"$status\"\n",
    );
    script
}

/// POSIX single-quote a byte string, preserving every byte exactly. `'` inside
/// becomes `'\''` and the whole value is wrapped in single quotes.
fn quote_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut quoted = Vec::with_capacity(bytes.len() + 2);
    quoted.push(b'\'');
    for &byte in bytes {
        if byte == b'\'' {
            quoted.extend_from_slice(b"'\\''");
        } else {
            quoted.push(byte);
        }
    }
    quoted.push(b'\'');
    quoted
}

/// Human-readable, single-quoted form of a string. For display only; never
/// feed this back to a shell.
fn shell_quote(value: &str) -> String {
    String::from_utf8_lossy(&quote_bytes(value.as_bytes())).into_owned()
}

fn shell_quote_path(path: &Path) -> String {
    String::from_utf8_lossy(&quote_bytes(path.as_os_str().as_bytes())).into_owned()
}

fn run_skill(cli: &SkillCli) -> Result<i32, String> {
    match cli.mode {
        SkillMode::Print => {
            print!("{SKILL}");
            Ok(0)
        }
        SkillMode::Install => {
            let dir = resolve_skill_dir(cli.dir.as_deref())?;
            let path = dir.join("SKILL.md");
            install_bytes(&path, SKILL.as_bytes())
        }
        SkillMode::Check => {
            let dir = resolve_skill_dir(cli.dir.as_deref())?;
            let path = dir.join("SKILL.md");
            let hint = format!("tmux-run skill --install {}", dir.display());
            check_file(&path, SKILL.as_bytes(), &hint)
        }
    }
}

fn resolve_skill_dir(explicit: Option<&Path>) -> Result<PathBuf, String> {
    resolve_skill_dir_from(explicit, env::var_os("HOME"))
}

fn resolve_skill_dir_from(
    explicit: Option<&Path>,
    home: Option<OsString>,
) -> Result<PathBuf, String> {
    if let Some(dir) = explicit {
        return Ok(dir.to_path_buf());
    }
    let home = home.filter(|value| !value.is_empty()).ok_or_else(|| {
        "HOME is not set; pass a directory with --install <DIR> or --check <DIR>".to_string()
    })?;
    Ok(PathBuf::from(home).join(".agents/skills/tmux-run"))
}

fn run_completion(cli: &CompletionCli) -> Result<i32, String> {
    let contents = completion_script(cli.shell);
    match cli.mode {
        CompletionMode::Print => {
            print!("{contents}");
            Ok(0)
        }
        CompletionMode::Install => {
            let path = completion_install_path(cli.shell)?;
            install_bytes(&path, contents.as_bytes())
        }
        CompletionMode::Check => {
            let path = completion_install_path(cli.shell)?;
            let hint = format!("tmux-run completion {} --install", cli.shell.word());
            check_file(&path, contents.as_bytes(), &hint)
        }
    }
}

fn completion_script(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => COMPLETION_BASH,
        Shell::Zsh => COMPLETION_ZSH,
        Shell::Fish => COMPLETION_FISH,
    }
}

fn completion_install_path(shell: Shell) -> Result<PathBuf, String> {
    completion_install_path_from(
        shell,
        env::var_os("XDG_DATA_HOME"),
        env::var_os("XDG_CONFIG_HOME"),
        env::var_os("HOME"),
    )
}

fn completion_install_path_from(
    shell: Shell,
    xdg_data_home: Option<OsString>,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, String> {
    let home = home.as_deref();
    match shell {
        Shell::Bash => {
            Ok(xdg_data_dir(xdg_data_home, home)?.join("bash-completion/completions/tmux-run"))
        }
        Shell::Zsh => Ok(xdg_data_dir(xdg_data_home, home)?.join("zsh/site-functions/_tmux-run")),
        Shell::Fish => {
            Ok(xdg_config_dir(xdg_config_home, home)?.join("fish/completions/tmux-run.fish"))
        }
    }
}

fn xdg_data_dir(xdg: Option<OsString>, home: Option<&OsStr>) -> Result<PathBuf, String> {
    if let Some(dir) = xdg.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = home
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "HOME is not set".to_string())?;
    Ok(PathBuf::from(home).join(".local/share"))
}

fn xdg_config_dir(xdg: Option<OsString>, home: Option<&OsStr>) -> Result<PathBuf, String> {
    if let Some(dir) = xdg.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = home
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "HOME is not set".to_string())?;
    Ok(PathBuf::from(home).join(".config"))
}

enum WriteOutcome {
    Created,
    Updated,
    UpToDate,
}

fn install_bytes(path: &Path, contents: &[u8]) -> Result<i32, String> {
    match write_if_changed(path, contents)? {
        WriteOutcome::Created => println!("created {}", path.display()),
        WriteOutcome::Updated => println!("updated {}", path.display()),
        WriteOutcome::UpToDate => println!("up to date {}", path.display()),
    }
    Ok(0)
}

fn write_if_changed(path: &Path, contents: &[u8]) -> Result<WriteOutcome, String> {
    match fs::read(path) {
        Ok(existing) if existing == contents => Ok(WriteOutcome::UpToDate),
        Ok(_) => {
            fs::write(path, contents)
                .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
            Ok(WriteOutcome::Updated)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
            }
            fs::write(path, contents)
                .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
            Ok(WriteOutcome::Created)
        }
        Err(err) => Err(format!("failed to read {}: {err}", path.display())),
    }
}

fn check_file(path: &Path, contents: &[u8], install_hint: &str) -> Result<i32, String> {
    match fs::read(path) {
        Ok(existing) if existing == contents => {
            println!("up to date {}", path.display());
            Ok(0)
        }
        _ => {
            eprintln!("error: {} is missing or out of date", path.display());
            eprintln!("       run: {install_hint}");
            Ok(1)
        }
    }
}

fn list_sessions(prefix: Option<&OsStr>) -> Vec<String> {
    list_sessions_in(&state_root(), prefix)
}

fn list_sessions_in(root: &Path, prefix: Option<&OsStr>) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return names;
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        if !entry.path().join("output.log").is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if let Some(prefix) = prefix
            && !name.as_bytes().starts_with(prefix.as_bytes())
        {
            continue;
        }
        names.push(name);
    }

    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parses_help_and_version_flags() {
        assert_eq!(
            parse_args(["--help"].into_iter().map(OsString::from)).unwrap(),
            CliAction::Help
        );
        assert_eq!(
            parse_args(["-h"].into_iter().map(OsString::from)).unwrap(),
            CliAction::Help
        );
        assert_eq!(
            parse_args(["--version"].into_iter().map(OsString::from)).unwrap(),
            CliAction::Version
        );
        assert_eq!(
            parse_args(["-V"].into_iter().map(OsString::from)).unwrap(),
            CliAction::Version
        );
    }

    #[test]
    fn rejects_missing_separator_and_command() {
        assert!(parse_args(["build", "cargo"].into_iter().map(OsString::from)).is_err());
        assert!(parse_args(["build", "--"].into_iter().map(OsString::from)).is_err());
        assert!(parse_args(Vec::<OsString>::new()).is_err());
    }

    #[test]
    fn normalizes_task_name_for_tmux_session() {
        assert_eq!(normalize_task_name("deploy: prod/eu"), "deploy_prod_eu");
        assert_eq!(normalize_task_name("!!!"), "task");
    }

    #[test]
    fn renders_script_preserving_argument_boundaries() {
        let script = String::from_utf8(render_script(&[
            OsString::from("printf"),
            OsString::from("%s\\n"),
            OsString::from("hello world"),
            OsString::from("it's ok"),
        ]))
        .unwrap();

        assert!(script.contains("cmd=( 'printf' '%s\\n' 'hello world' 'it'\\''s ok' )"));
        assert!(script.contains("printf '__DONE__:%s\\n' \"$status\""));
    }

    #[test]
    fn renders_script_byte_for_byte_for_non_ascii_and_quotes() {
        let script = String::from_utf8(render_script(&[
            OsString::from("printf"),
            OsString::from("café"),
            OsString::from(""),
            OsString::from("a'b"),
        ]))
        .unwrap();

        assert!(script.contains("'café'"));
        assert!(script.contains("''"));
        assert!(script.contains("'a'\\''b'"));
        assert!(!script.contains("cafÃ©"));
    }

    #[test]
    fn quote_bytes_preserves_bytes_and_escapes_single_quotes() {
        assert_eq!(quote_bytes(b""), b"''");
        assert_eq!(quote_bytes(b"caf\xc3\xa9"), b"'caf\xc3\xa9'");
        assert_eq!(quote_bytes(b"a'b"), b"'a'\\''b'");
    }

    #[test]
    fn shell_quote_for_display_handles_empty_and_quotes() {
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(
            shell_quote_path(Path::new(OsStr::new("/tmp/a b"))),
            "'/tmp/a b'"
        );
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
    fn parses_skill_subcommand() {
        assert_eq!(
            parse_args(["skill"].into_iter().map(OsString::from)).unwrap(),
            CliAction::Skill(SkillCli {
                mode: SkillMode::Print,
                dir: None
            })
        );
        assert_eq!(
            parse_args(["skill", "--install"].into_iter().map(OsString::from)).unwrap(),
            CliAction::Skill(SkillCli {
                mode: SkillMode::Install,
                dir: None
            })
        );
        assert_eq!(
            parse_args(
                ["skill", "--install", "/tmp/s"]
                    .into_iter()
                    .map(OsString::from)
            )
            .unwrap(),
            CliAction::Skill(SkillCli {
                mode: SkillMode::Install,
                dir: Some(PathBuf::from("/tmp/s"))
            })
        );
        assert_eq!(
            parse_args(
                ["skill", "--check", "/tmp/s"]
                    .into_iter()
                    .map(OsString::from)
            )
            .unwrap(),
            CliAction::Skill(SkillCli {
                mode: SkillMode::Check,
                dir: Some(PathBuf::from("/tmp/s"))
            })
        );
    }

    #[test]
    fn parses_completion_subcommand() {
        assert_eq!(
            parse_args(["completion", "bash"].into_iter().map(OsString::from)).unwrap(),
            CliAction::Completion(CompletionCli {
                shell: Shell::Bash,
                mode: CompletionMode::Print
            })
        );
        assert_eq!(
            parse_args(
                ["completion", "fish", "--install"]
                    .into_iter()
                    .map(OsString::from)
            )
            .unwrap(),
            CliAction::Completion(CompletionCli {
                shell: Shell::Fish,
                mode: CompletionMode::Install
            })
        );
        assert_eq!(
            parse_args(
                ["completion", "zsh", "--check"]
                    .into_iter()
                    .map(OsString::from)
            )
            .unwrap(),
            CliAction::Completion(CompletionCli {
                shell: Shell::Zsh,
                mode: CompletionMode::Check
            })
        );
    }

    #[test]
    fn rejects_usage_errors() {
        assert!(parse_args(["completion"].into_iter().map(OsString::from)).is_err());
        assert!(parse_args(["completion", "nope"].into_iter().map(OsString::from)).is_err());
        assert!(
            parse_args(
                ["skill", "--install", "--check"]
                    .into_iter()
                    .map(OsString::from)
            )
            .is_err()
        );
        assert!(parse_args(["__complete"].into_iter().map(OsString::from)).is_err());
        assert!(parse_args(["__complete", "other"].into_iter().map(OsString::from)).is_err());
        assert!(
            parse_args(
                ["__complete", "sessions", "a", "b"]
                    .into_iter()
                    .map(OsString::from)
            )
            .is_err()
        );
    }

    #[test]
    fn skill_dir_requires_home_without_explicit_dir() {
        assert_eq!(
            resolve_skill_dir_from(None, Some(OsString::from("/home/u"))).unwrap(),
            PathBuf::from("/home/u/.agents/skills/tmux-run")
        );
        assert!(resolve_skill_dir_from(None, None).is_err());
        assert_eq!(
            resolve_skill_dir_from(Some(Path::new("/custom")), None).unwrap(),
            PathBuf::from("/custom")
        );
    }

    #[test]
    fn resolves_completion_install_paths() {
        assert_eq!(
            completion_install_path_from(
                Shell::Bash,
                Some(OsString::from("/data")),
                Some(OsString::from("/cfg")),
                Some(OsString::from("/home/u")),
            )
            .unwrap(),
            PathBuf::from("/data/bash-completion/completions/tmux-run")
        );
        assert_eq!(
            completion_install_path_from(Shell::Zsh, None, None, Some(OsString::from("/home/u")),)
                .unwrap(),
            PathBuf::from("/home/u/.local/share/zsh/site-functions/_tmux-run")
        );
        assert_eq!(
            completion_install_path_from(
                Shell::Fish,
                None,
                Some(OsString::from("/cfg")),
                Some(OsString::from("/home/u")),
            )
            .unwrap(),
            PathBuf::from("/cfg/fish/completions/tmux-run.fish")
        );
        assert!(completion_install_path_from(Shell::Bash, None, None, None).is_err());
    }

    #[test]
    fn skill_install_then_check_cycle() {
        let dir = test_dir("skill");
        let install = SkillCli {
            mode: SkillMode::Install,
            dir: Some(dir.clone()),
        };
        let check = SkillCli {
            mode: SkillMode::Check,
            dir: Some(dir.clone()),
        };
        let path = dir.join("SKILL.md");

        assert_eq!(run_skill(&install).unwrap(), 0);
        assert_eq!(fs::read(&path).unwrap(), SKILL.as_bytes());

        assert_eq!(run_skill(&check).unwrap(), 0);

        let mut edited = SKILL.as_bytes().to_vec();
        edited[0] ^= 0xff;
        fs::write(&path, &edited).unwrap();
        assert_eq!(run_skill(&check).unwrap(), 1);

        fs::remove_file(&path).unwrap();
        assert_eq!(run_skill(&check).unwrap(), 1);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn completion_install_then_check_cycle() {
        let dir = test_dir("completion");
        let path = dir.join("tmux-run.bash");
        let contents = COMPLETION_BASH.as_bytes();

        assert_eq!(install_bytes(&path, contents).unwrap(), 0);
        assert_eq!(fs::read(&path).unwrap(), contents);
        assert_eq!(
            check_file(&path, contents, "tmux-run completion bash --install").unwrap(),
            0
        );

        let mut edited = contents.to_vec();
        edited[0] ^= 0xff;
        fs::write(&path, &edited).unwrap();
        assert_eq!(
            check_file(&path, contents, "tmux-run completion bash --install").unwrap(),
            1
        );

        fs::remove_file(&path).unwrap();
        assert_eq!(
            check_file(&path, contents, "tmux-run completion bash --install").unwrap(),
            1
        );

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn embedded_skill_declares_matching_name() {
        let frontmatter = SKILL
            .split("---")
            .nth(1)
            .expect("skill must have YAML frontmatter");
        assert!(
            frontmatter
                .lines()
                .any(|line| line.trim() == "name: tmux-run"),
            "skill frontmatter must declare `name: tmux-run`"
        );
    }

    // Fails when a subcommand or flag is added without updating the completion scripts.
    #[test]
    fn completion_scripts_contain_required_tokens() {
        for (label, script) in [
            ("bash", COMPLETION_BASH),
            ("zsh", COMPLETION_ZSH),
            ("fish", COMPLETION_FISH),
        ] {
            for token in [
                "wait",
                "skill",
                "completion",
                "__complete",
                "--help",
                "--version",
            ] {
                assert!(
                    script.contains(token),
                    "{label} script must contain `{token}`"
                );
            }
        }
    }

    #[test]
    fn lists_sessions_with_prefix_filtering() {
        let root = test_dir("complete");
        let make = |name: &str, log: bool| {
            let dir = root.join(name);
            fs::create_dir_all(&dir).unwrap();
            if log {
                fs::write(dir.join("output.log"), "x").unwrap();
            }
        };
        make("build_1", true);
        make("deploy_2", true);
        make("no-log_3", false);
        make("build_2", true);

        assert_eq!(
            list_sessions_in(&root, None),
            vec![
                "build_1".to_string(),
                "build_2".to_string(),
                "deploy_2".to_string()
            ]
        );
        assert_eq!(
            list_sessions_in(&root, Some(OsStr::new("build"))),
            vec!["build_1".to_string(), "build_2".to_string()]
        );
        assert_eq!(
            list_sessions_in(&root, Some(OsStr::new("nope"))),
            Vec::<String>::new()
        );

        fs::remove_dir_all(root).unwrap();
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
        fs::write(&log, "no trailing newline__DONE__:0\n").unwrap();
        assert_eq!(read_done_marker(&log).unwrap(), Some(0));
        fs::write(&log, "x__DONE__:7\n").unwrap();
        assert_eq!(read_done_marker(&log).unwrap(), Some(7));
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
}

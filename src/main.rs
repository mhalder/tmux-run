use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

const DONE_MARKER_PREFIX: &str = "__DONE__:";
const HELP: &str = "Usage: tmux-run <task-name> -- <command> [args...]\n\nStarts <command> in a detached tmux session and writes stdout/stderr to a log file.\n\nArguments:\n  <task-name>        Name used to build the tmux session name\n  --                 Separates tmux-run arguments from the command\n  <command> [args...] Command and arguments to run under bash\n\nOutput:\n  session            tmux session name\n  log                path to combined stdout/stderr log\n  completion marker  __DONE__:<status> appended when the command exits\n\nExamples:\n  tmux-run build -- cargo test\n  tmux-run deploy -- bash -lc 'echo start; ./deploy.sh'\n";

#[derive(Debug, PartialEq, Eq)]
enum CliAction {
    Help,
    Run(Cli),
}

#[derive(Debug, PartialEq, Eq)]
struct Cli {
    task_name: String,
    command: Vec<OsString>,
}

#[derive(Debug)]
struct RuntimePaths {
    session_name: String,
    script_path: PathBuf,
    log_path: PathBuf,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        eprintln!("usage: tmux-run <task-name> -- <command> [args...]");
        process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let CliAction::Run(cli) = parse_args(env::args_os().skip(1))? else {
        print!("{HELP}");
        return Ok(());
    };

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
    let task_name = args
        .next()
        .ok_or_else(|| "missing task name".to_string())?
        .into_string()
        .map_err(|_| "task name must be valid UTF-8".to_string())?;

    if task_name == "--help" || task_name == "-h" {
        return Ok(CliAction::Help);
    }

    if task_name.is_empty() {
        return Err("task name must not be empty".to_string());
    }

    match args.next() {
        Some(separator) if separator == "--" => {}
        Some(_) => return Err("missing `--` before command".to_string()),
        None => return Err("missing `--` before command".to_string()),
    }

    let command: Vec<OsString> = args.collect();
    if command.is_empty() {
        return Err("missing command after `--`".to_string());
    }

    Ok(CliAction::Run(Cli { task_name, command }))
}

fn build_runtime_paths(task_name: &str) -> io::Result<RuntimePaths> {
    let unique = unique_suffix();
    let normalized = normalize_task_name(task_name);
    let session_name = format!("{normalized}_{unique}");
    let dir = env::temp_dir().join(format!("tmux-run-{session_name}"));
    fs::create_dir_all(&dir)?;

    Ok(RuntimePaths {
        session_name,
        script_path: dir.join("run.sh"),
        log_path: dir.join("output.log"),
    })
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
    fn quotes_empty_and_single_quote_bytes() {
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(
            shell_quote_path(Path::new(OsStr::new("/tmp/a b"))),
            "'/tmp/a b'"
        );
    }
}

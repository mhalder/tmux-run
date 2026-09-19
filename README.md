# tmux-run

`tmux-run` starts a command in a detached tmux session, writes stdout and stderr to a log file, and appends a completion marker when the command exits.

## Requirements

- `tmux` installed and on `PATH`
- `bash` (the command runs under bash)

## Install

```sh
cargo install --path .
```

## Usage

```sh
tmux-run <task-name> -- <command> [args...]
tmux-run wait <session-name> [--timeout <seconds>]
tmux-run --help
```

Example:

```sh
tmux-run build -- cargo test
tmux-run wait build_1234-5678 --timeout 600
```

## Wait for completion

`tmux-run wait` blocks until the log records the completion marker, then exits with the recorded status:

- the command's exit status
- `124` if `--timeout` elapsed
- `3` if the session ended or never existed without a marker
- `2` on a usage error

## Output

`tmux-run` starts the session and exits immediately; it does not wait for the command to finish. On startup it prints:

- the tmux session name
- the log path
- a `tmux-run wait` command for the session
- the completion marker format, `__DONE__:<status>`
- example attach and log-follow commands

The marker is appended to the log when the command exits, with the command's exit status. Command stdout and stderr are combined into the log.

The task name is normalized for tmux and combined with the process id and timestamp to make the session name unique. The log and the generated bash script live in `tmux-run/<session>` under `$XDG_STATE_HOME`, then `~/.local/state`, then the system temp directory; command arguments are written into the script with argument boundaries preserved, rather than interpolated directly into the tmux command line. State is never deleted automatically.

## Agent skill

[`docs/SKILL.md`](docs/SKILL.md) is a [pi](https://pi.dev) skill that teaches a coding agent to run long commands with tmux-run: starting a task, waiting for it, reading its log and completion marker, and closing the session afterwards. Load it by path, for example:

```sh
pi --skill docs/SKILL.md
```
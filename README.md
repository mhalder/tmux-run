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
tmux-run --help
```

Example:

```sh
tmux-run build -- cargo test
```

## Output

`tmux-run` starts the session and exits immediately; it does not wait for the command to finish. On startup it prints:

- the tmux session name
- the log path
- the completion marker format, `__DONE__:<status>`
- example attach and log-follow commands

The marker is appended to the log when the command exits, with the command's exit status. Command stdout and stderr are combined into the log.

The task name is normalized for tmux and combined with the process id and timestamp to make the session name unique. The log and a temporary bash script live under the system temp directory in `tmux-run-<session>`; command arguments are written into the script with argument boundaries preserved, rather than interpolated directly into the tmux command line.

## Agent skill

[`docs/SKILL.md`](docs/SKILL.md) is a [pi](https://pi.dev) skill that teaches a coding agent to run long commands with tmux-run: starting a task, reading its log and completion marker, and closing the session afterwards. Load it by path, for example:

```sh
pi --skill docs/SKILL.md
```
# tmux-run

`tmux-run` starts a command in a detached tmux session, writes stdout and stderr to a log file, and appends a completion marker when the command exits.

## Requirements

- `tmux` installed and on `PATH`
- `bash` (the command runs under bash)

## Install

```sh
cargo build --release
# the binary is at target/release/tmux-run
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
# tmux-run

`tmux-run` starts a command in a detached tmux session, writes stdout and stderr to a predictable log file, and appends a completion marker when the command exits.

## Usage

```sh
tmux-run <task-name> -- <command> [args...]
tmux-run --help
```

Example:

```sh
tmux-run build -- cargo test
```

Output includes:

- the tmux session name
- the log path
- the completion marker format, `__DONE__:<status>`
- example attach and log-follow commands

The task name is normalized for tmux and combined with the process id and timestamp to make the session name unique. Command arguments are written into a temporary bash script with argument boundaries preserved, rather than interpolated directly into the tmux command line.

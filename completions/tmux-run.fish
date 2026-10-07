# tmux-run completions for fish
# Subcommands: list show wait skill completion __complete
# Top-level options: --help --version
# Subcommand flags: --json --lines

complete -c tmux-run -n '__fish_use_subcommand' -a list -d 'List all sessions and their state' -f
complete -c tmux-run -n '__fish_use_subcommand' -a show -d 'Show a session and the tail of its log' -f
complete -c tmux-run -n '__fish_use_subcommand' -a wait -d 'Wait for a running task' -f
complete -c tmux-run -n '__fish_use_subcommand' -a skill -d 'Print or manage the agent skill' -f
complete -c tmux-run -n '__fish_use_subcommand' -a completion -d 'Print or install shell completions' -f
complete -c tmux-run -n '__fish_use_subcommand' -a __complete -d 'Internal completion helper' -f
complete -c tmux-run -n '__fish_use_subcommand' -a '--help' -d 'Print help and exit' -f
complete -c tmux-run -n '__fish_use_subcommand' -a '--version' -d 'Print version and exit' -f

complete -c tmux-run -n '__fish_seen_subcommand_from wait' -l timeout -r -d 'Give up after this many seconds' -f
complete -c tmux-run -n '__fish_seen_subcommand_from wait' -a '(tmux-run __complete sessions (commandline -ct))' -d 'Session name' -f

complete -c tmux-run -n '__fish_seen_subcommand_from list' -l json -d 'Print the list as JSON' -f

complete -c tmux-run -n '__fish_seen_subcommand_from show' -l lines -r -d 'Show the last N log lines' -f
complete -c tmux-run -n '__fish_seen_subcommand_from show' -l json -d 'Print as JSON' -f
complete -c tmux-run -n '__fish_seen_subcommand_from show' -a '(tmux-run __complete sessions (commandline -ct))' -d 'Session name' -f

complete -c tmux-run -n '__fish_seen_subcommand_from skill; and not __fish_seen_subcommand_from --install --check' -a '--install' -d 'Install the skill' -f
complete -c tmux-run -n '__fish_seen_subcommand_from skill; and not __fish_seen_subcommand_from --install --check' -a '--check' -d 'Check the installed skill' -f
complete -c tmux-run -n '__fish_seen_subcommand_from skill; and __fish_seen_subcommand_from --install --check' -f

complete -c tmux-run -n '__fish_seen_subcommand_from completion; and not __fish_seen_subcommand_from bash zsh fish' -a 'bash zsh fish' -d 'Shell' -f
complete -c tmux-run -n '__fish_seen_subcommand_from completion; and __fish_seen_subcommand_from bash zsh fish' -a '--install' -d 'Install the completion' -f
complete -c tmux-run -n '__fish_seen_subcommand_from completion; and __fish_seen_subcommand_from bash zsh fish' -a '--check' -d 'Check the installed completion' -f

complete -c tmux-run -n '__fish_seen_subcommand_from __complete' -a sessions -d 'Subcommand' -f

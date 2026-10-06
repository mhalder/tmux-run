# tmux-run completions for fish
# Subcommands: wait skill completion __complete
# Top-level options: --help --version

complete -c tmux-run -n '__fish_use_subcommand' -a wait -d 'Wait for a running task'
complete -c tmux-run -n '__fish_use_subcommand' -a skill -d 'Print or manage the agent skill'
complete -c tmux-run -n '__fish_use_subcommand' -a completion -d 'Print or install shell completions'
complete -c tmux-run -n '__fish_use_subcommand' -a __complete -d 'Internal completion helper'
complete -c tmux-run -n '__fish_use_subcommand' -l help -s h -d 'Print help and exit'
complete -c tmux-run -n '__fish_use_subcommand' -l version -s V -d 'Print version and exit'

complete -c tmux-run -n '__fish_seen_subcommand_from wait' -l timeout -r -d 'Give up after this many seconds'
complete -c tmux-run -n '__fish_seen_subcommand_from wait' -a '(tmux-run __complete sessions (commandline -ct))' -d 'Session name'

complete -c tmux-run -n '__fish_seen_subcommand_from skill' -l install -d 'Install the skill'
complete -c tmux-run -n '__fish_seen_subcommand_from skill' -l check -d 'Check the installed skill'

complete -c tmux-run -n '__fish_seen_subcommand_from completion' -a 'bash zsh fish' -d 'Shell'
complete -c tmux-run -n '__fish_seen_subcommand_from completion' -l install -d 'Install the completion'
complete -c tmux-run -n '__fish_seen_subcommand_from completion' -l check -d 'Check the installed completion'

complete -c tmux-run -n '__fish_seen_subcommand_from __complete' -a sessions -d 'Subcommand'

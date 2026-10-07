# bash completion for tmux-run
# Subcommands: list show wait skill completion __complete
# Top-level options: --help --version

_tmux_run() {
    local cur prev
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"

    local subcommands="list show wait skill completion __complete"
    local top_options="--help --version"
    local skill_options="--install --check"
    local completion_options="--install --check"
    local shells="bash zsh fish"

    local i cmd=""
    for ((i = 1; i < COMP_CWORD; i++)); do
        case "${COMP_WORDS[i]}" in
            list|show|wait|skill|completion|__complete)
                cmd="${COMP_WORDS[i]}"
                ;;
        esac
    done

    case "$cmd" in
        list)
            COMPREPLY=( $(compgen -W "--json" -- "$cur") )
            ;;
        show)
            if [[ "$prev" == "--lines" ]]; then
                COMPREPLY=()
                return
            fi
            local sessions
            sessions="$(tmux-run __complete sessions "$cur" 2>/dev/null)"
            COMPREPLY=( $(compgen -W "--lines --json $sessions" -- "$cur") )
            ;;
        wait)
            if [[ "$prev" == "--timeout" ]]; then
                COMPREPLY=()
                return
            fi
            local sessions
            sessions="$(tmux-run __complete sessions "$cur" 2>/dev/null)"
            COMPREPLY=( $(compgen -W "--timeout $sessions" -- "$cur") )
            ;;
        skill)
            COMPREPLY=( $(compgen -W "$skill_options" -- "$cur") )
            ;;
        completion)
            if [[ "$prev" == "completion" ]]; then
                COMPREPLY=( $(compgen -W "$shells" -- "$cur") )
            else
                COMPREPLY=( $(compgen -W "$completion_options" -- "$cur") )
            fi
            ;;
        __complete)
            COMPREPLY=( $(compgen -W "sessions" -- "$cur") )
            ;;
        *)
            COMPREPLY=( $(compgen -W "$subcommands $top_options" -- "$cur") )
            ;;
    esac
}

complete -F _tmux_run tmux-run

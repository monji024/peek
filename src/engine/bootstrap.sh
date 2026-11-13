# https://github.com/monji024

if [[ -z "${__PEEK_AGENT_INIT:-}" ]]; then
    __PEEK_AGENT_INIT=1

    exec {__PEEK_STATE_FD}> "$PEEK_STATE_FIFO"
    exec {__PEEK_LOCK_FD}> "$PEEK_LOCK_FILE"
    exec {__PEEK_SAVED_IN}<&0
    exec {__PEEK_SAVED_OUT}>&1
    exec {__PEEK_SAVED_ERR}>&2

    __PEEK_ROOT_PID=$BASHPID
    __PEEK_LOCK_PID=$BASHPID
    __PEEK_CMD_PID=0
    __PEEK_NONCE="$PEEK_NONCE"
    __PEEK_BOOT="$PEEK_BOOTSTRAP"
    __PEEK_TARGET_SCRIPT="$PEEK_TARGET"
    __PEEK_SESSION_DIR="${PEEK_SESSION_DIR:-}"
    __PEEK_DEFAULT_CMD_FIFO="$PEEK_CMD_FIFO"
    __PEEK_MODE="${PEEK_INIT_MODE:-STEP}"
    __PEEK_TARGET_DEPTH=0
    __PEEK_ERR_TRAP="${PEEK_INIT_ERR_TRAP:-0}"
    __PEEK_BPS="${PEEK_INIT_BPS:-}"
    __PEEK_COND="${PEEK_INIT_COND:-}"
    __PEEK_TTY_ACTIVE=0
    __PEEK_IN_INTERNAL=0
    __PEEK_USER_EXTDEBUG=0
    __PEEK_USER_DEBUG_TRAP=""
    __PEEK_USER_EXIT_TRAP=""
    __PEEK_HAS_FLOCK=0
    command -v flock >/dev/null 2>&1 && __PEEK_HAS_FLOCK=1

    if [[ -n "$__PEEK_SESSION_DIR" ]]; then
        export PEEK_STATE_FIFO PEEK_LOCK_FILE PEEK_BOOTSTRAP PEEK_SESSION_DIR PEEK_NONCE PEEK_CMD_FIFO
        export PEEK_INIT_BPS="$__PEEK_BPS"
        export PEEK_INIT_ERR_TRAP="$__PEEK_ERR_TRAP"
        export PEEK_INIT_COND="$__PEEK_COND"
        export BASH_ENV="$__PEEK_BOOT"
    else
        unset PEEK_STATE_FIFO PEEK_CMD_FIFO PEEK_LOCK_FILE PEEK_BOOTSTRAP PEEK_TARGET PEEK_INIT_BPS PEEK_INIT_ERR_TRAP PEEK_NONCE PEEK_SESSION_DIR PEEK_INIT_MODE PEEK_INIT_COND
    fi
fi

__peek_ensure_cmd_fd() {
    if (( __PEEK_CMD_PID != BASHPID )); then
        local __pfifo="$__PEEK_DEFAULT_CMD_FIFO"
        if [[ -n "$__PEEK_SESSION_DIR" && -p "$__PEEK_SESSION_DIR/cmd.$BASHPID.fifo" ]]; then
            __pfifo="$__PEEK_SESSION_DIR/cmd.$BASHPID.fifo"
        fi
        exec {__PEEK_CMD_FD}<> "$__pfifo"
        __PEEK_CMD_PID=$BASHPID
    fi
}

__peek_lock() {
    if (( __PEEK_HAS_FLOCK == 1 )); then
        if (( __PEEK_LOCK_PID != BASHPID )); then
            exec {__PEEK_LOCK_FD}> "$PEEK_LOCK_FILE" 2>/dev/null || true
            __PEEK_LOCK_PID=$BASHPID
        fi
        flock -x "$__PEEK_LOCK_FD" 2>/dev/null || true
    fi
}

__peek_unlock() {
    if (( __PEEK_HAS_FLOCK == 1 )); then
        flock -u "$__PEEK_LOCK_FD" 2>/dev/null || true
    fi
}

__peek_needs_tty() {
    local __c="$1"
    [[ "$__c" =~ (^|[[:space:]|;&\(])(sudo[[:space:]]+)?(vim|nvim|vi|nano|emacs|fzf|sk|dialog|whiptail|gum|less|more|man|htop|top|btop|lazygit|tig|ssh)([[:space:]|;&\)]|$) ]]
}

__peek_emit_proc_start() {
    local __role="$1"
    local __script="${2//$'\t'/ }"
    __script="${__script//$'\n'/ }"
    __peek_lock
    printf "PROC_START\t%s\t%s\t%s\t%s\t%s\n" \
        "$__PEEK_NONCE" "$BASHPID" "$PPID" "$__role" "$__script" >&"$__PEEK_STATE_FD" 2>/dev/null || true
    __peek_unlock
}

__peek_emit_proc_exit() {
    local __code="$1"
    __peek_lock
    printf "PROC_EXIT\t%s\t%s\t%s\n" \
        "$__PEEK_NONCE" "$BASHPID" "$__code" >&"$__PEEK_STATE_FD" 2>/dev/null || true
    __peek_unlock
}

__peek_emit_frame() {
    local __kind="$1"
    local __src="${2//$'\t'/ }"
    __src="${__src//$'\n'/ }"
    local __line="$3"
    local __rc="$4"
    local __pipe_str="$5"
    local __cmd="${6//$'\n'/ }"
    local __pwd="${PWD//$'\n'/ }"

    printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n" \
        "$__kind" "$__src" "$__line" "$__rc" "$BASH_SUBSHELL" "$__pipe_str" "$BASHPID" "$PPID" >&"$__PEEK_STATE_FD"
    printf "PWD:%s\n" "$__pwd" >&"$__PEEK_STATE_FD"
    printf "CMD:%s\n" "$__cmd" >&"$__PEEK_STATE_FD"

    local __idx
    local __max_idx=$(( ${#FUNCNAME[@]} - 2 ))
    for (( __idx=1; __idx < __max_idx; __idx++ )); do
        local __fn="${FUNCNAME[$__idx]}"
        local __fsrc="${BASH_SOURCE[$__idx]:-$__src}"
        local __flin="${BASH_LINENO[$((__idx-1))]:-0}"
        if [[ "$__fsrc" != "$__PEEK_BOOT" ]]; then
            printf "FRAME:%s\t%s\t%s\n" "$__fn" "$__fsrc" "$__flin" >&"$__PEEK_STATE_FD"
        fi
    done

    printf "__PEEK_VARS_%s__\n" "$__PEEK_NONCE" >&"$__PEEK_STATE_FD"
    declare -p >&"$__PEEK_STATE_FD" 2>/dev/null
    printf "\n__PEEK_END_%s__\n" "$__PEEK_NONCE" >&"$__PEEK_STATE_FD"
}

__peek_restore_env() {
    local __flags="$1"
    local __had_pipefail="$2"
    local __had_nocase="$3"
    local __had_extglob="$4"
    local __had_nullglob="$5"

    if (( __had_nocase == 1 )); then builtin shopt -s nocasematch; fi
    if (( __had_extglob == 1 )); then builtin shopt -s extglob; fi
    if (( __had_nullglob == 1 )); then builtin shopt -s nullglob; fi
    if (( __had_pipefail == 1 )); then set -o pipefail; fi
    if [[ "$__flags" == *u* ]]; then set -u; fi
    if [[ "$__flags" == *f* ]]; then set -f; fi
    if [[ "$__flags" == *x* ]]; then set -x; fi
    if [[ "$__flags" == *v* ]]; then set -v; fi
    if [[ "$__flags" == *e* ]]; then set -e; fi
    return 0
}

__peek_reclaim_tty() {
    if (( __PEEK_TTY_ACTIVE == 1 )); then
        exec 0<&"$__PEEK_SAVED_IN" 1>&"$__PEEK_SAVED_OUT" 2>&"$__PEEK_SAVED_ERR"
        __PEEK_TTY_ACTIVE=0
        printf "TTY_DONE\t%s\t%s\n" "$__PEEK_NONCE" "$BASHPID" >&"$__PEEK_STATE_FD"
    fi
}

__peek_set_rc() {
    return "$1"
}

shopt() {
    local __saved_rc=$?
    __PEEK_IN_INTERNAL=1
    local __sh_flags="$-"
    set +eu +f
    local __mode="" __quiet=0 __print=0 __opt
    local __other=()
    local __has_extdebug=0
    for __opt in "$@"; do
        case "$__opt" in
            -s) __mode="s"; __other+=("$__opt") ;;
            -u) __mode="u"; __other+=("$__opt") ;;
            -q) __quiet=1; __other+=("$__opt") ;;
            -p) __print=1; __other+=("$__opt") ;;
            -sq|-qs) __mode="s"; __quiet=1; __other+=("$__opt") ;;
            -uq|-qu) __mode="u"; __quiet=1; __other+=("$__opt") ;;
            extdebug)
                __has_extdebug=1
                ;;
            *)
                __other+=("$__opt")
                ;;
        esac
    done

    local __ret=0
    if (( __has_extdebug == 1 )); then
        if [[ "$__mode" == "s" ]]; then
            __PEEK_USER_EXTDEBUG=1
        elif [[ "$__mode" == "u" ]]; then
            __PEEK_USER_EXTDEBUG=0
        elif (( __quiet == 1 )); then
            if (( __PEEK_USER_EXTDEBUG == 0 )); then __ret=1; fi
        elif (( __print == 1 )); then
            if (( __PEEK_USER_EXTDEBUG == 1 )); then
                printf "shopt -s extdebug\n"
            else
                printf "shopt -u extdebug\n"
            fi
        else
            if (( __PEEK_USER_EXTDEBUG == 1 )); then
                printf "extdebug       \ton\n"
            else
                printf "extdebug       \toff\n"
            fi
            if (( __PEEK_USER_EXTDEBUG == 0 )); then __ret=1; fi
        fi
        if (( $# > 2 )) || { (( $# == 2 )) && [[ -z "$__mode" && $__quiet == 0 && $__print == 0 ]]; }; then
            builtin shopt "${__other[@]}" || __ret=$?
        fi
        if [[ "$__sh_flags" == *u* ]]; then set -u; fi
        if [[ "$__sh_flags" == *f* ]]; then set -f; fi
        if [[ "$__sh_flags" == *e* ]]; then set -e; fi
        __PEEK_IN_INTERNAL=0
        return $__ret
    fi

    builtin shopt "$@" || __ret=$?
    builtin shopt -s extdebug 2>/dev/null || true
    if [[ "$__sh_flags" == *u* ]]; then set -u; fi
    if [[ "$__sh_flags" == *f* ]]; then set -f; fi
    if [[ "$__sh_flags" == *e* ]]; then set -e; fi
    __PEEK_IN_INTERNAL=0
    return $__ret
}

trap() {
    local __saved_rc=$?
    __PEEK_IN_INTERNAL=1
    local __tr_flags="$-"
    set +eu +f
    local __args=("$@")
    local __has_debug=0
    local __has_exit=0
    local __other_sigs=()

    if (( ${#__args[@]} >= 2 )) && [[ "${__args[0]}" != -* ]]; then
        local __action="${__args[0]}"
        if [[ "$__action" == "-" ]]; then __action=""; fi
        local __i
        for (( __i=1; __i < ${#__args[@]}; __i++ )); do
            case "${__args[$__i]}" in
                DEBUG|debug)
                    __has_debug=1
                    ;;
                EXIT|exit|0)
                    __has_exit=1
                    ;;
                *)
                    __other_sigs+=("${__args[$__i]}")
                    ;;
            esac
        done
        if (( __has_debug == 1 || __has_exit == 1 )); then
            if (( __has_debug == 1 )); then
                __PEEK_USER_DEBUG_TRAP="$__action"
                builtin trap '__peek_hook "$LINENO" "$BASH_COMMAND" "$?" "${PIPESTATUS[*]}"' DEBUG
            fi
            if (( __has_exit == 1 )); then
                __PEEK_USER_EXIT_TRAP="$__action"
                builtin trap '__peek_on_exit' EXIT
            fi
            if (( ${#__other_sigs[@]} > 0 )); then
                builtin trap "$__action" "${__other_sigs[@]}"
            fi
            if [[ "$__tr_flags" == *u* ]]; then set -u; fi
            if [[ "$__tr_flags" == *f* ]]; then set -f; fi
            if [[ "$__tr_flags" == *e* ]]; then set -e; fi
            __PEEK_IN_INTERNAL=0
            return 0
        fi
    fi

    local __ret=0
    builtin trap "$@" || __ret=$?
    if [[ "$__tr_flags" == *u* ]]; then set -u; fi
    if [[ "$__tr_flags" == *f* ]]; then set -f; fi
    if [[ "$__tr_flags" == *e* ]]; then set -e; fi
    __PEEK_IN_INTERNAL=0
    return $__ret
}

__peek_on_exit() {
    local __exit_rc=$?
    __PEEK_IN_INTERNAL=1
    set +eu +f
    __peek_reclaim_tty
    if [[ -n "$__PEEK_USER_EXIT_TRAP" ]]; then
        __peek_set_rc "$__exit_rc"
        eval "$__PEEK_USER_EXIT_TRAP"
        __exit_rc=$?
    fi
    __peek_emit_proc_exit "$__exit_rc"
    return "$__exit_rc"
}

__peek_hook() {
    local __rc="${3:-$?}"
    local __pipe_str="${4:-0}"
    if (( __PEEK_IN_INTERNAL == 1 )); then
        return 0
    fi
    local __freeze_sec=$SECONDS
    local __flags="$-"
    local __had_pipefail=0
    local __had_nocase=0
    local __had_extglob=0
    local __had_nullglob=0

    [[ -o pipefail ]] && __had_pipefail=1
    builtin shopt -q nocasematch && __had_nocase=1
    builtin shopt -q extglob && __had_extglob=1
    builtin shopt -q nullglob && __had_nullglob=1

    set +eu +f +x +v
    builtin shopt -u nocasematch nullglob

    __peek_reclaim_tty

    local __src="${BASH_SOURCE[1]}"
    if [[ -z "$__src" || "$__src" == "$__PEEK_BOOT" ]]; then
        SECONDS=$__freeze_sec
        __peek_restore_env "$__flags" "$__had_pipefail" "$__had_nocase" "$__had_extglob" "$__had_nullglob"
        __peek_set_rc "$__rc"
        return 0
    fi

    local __line="$1"
    local __cmd="$2"
    local __depth=$(( ${#FUNCNAME[@]} - 3 ))
    (( __depth < 0 )) && __depth=0

    local __in_subshell=0
    (( BASHPID != __PEEK_ROOT_PID )) && __in_subshell=1

    if [[ -n "$__PEEK_USER_DEBUG_TRAP" ]]; then
        __PEEK_IN_INTERNAL=1
        __peek_set_rc "$__rc"
        eval "$__PEEK_USER_DEBUG_TRAP"
        local __user_trap_rc=$?
        __PEEK_IN_INTERNAL=0
        if (( __PEEK_USER_EXTDEBUG == 1 && __user_trap_rc != 0 )); then
            SECONDS=$__freeze_sec
            __peek_restore_env "$__flags" "$__had_pipefail" "$__had_nocase" "$__had_extglob" "$__had_nullglob"
            return "$__user_trap_rc"
        fi
    fi

    local __stop=0
    if [[ "$__PEEK_MODE" == "STEP" ]]; then
        __stop=1
    elif (( __PEEK_ERR_TRAP == 1 && __rc != 0 )); then
        __stop=1
    elif [[ " $__PEEK_BPS " == *" $__src:$__line "* ]]; then
        __stop=1
    elif (( __in_subshell == 0 )) && [[ "$__PEEK_MODE" == "NEXT" ]] && (( __depth <= __PEEK_TARGET_DEPTH )); then
        __stop=1
    elif (( __in_subshell == 0 )) && [[ "$__PEEK_MODE" == "OUT" ]] && (( __depth < __PEEK_TARGET_DEPTH )); then
        __stop=1
    elif [[ -n "$__PEEK_COND" ]]; then
        __PEEK_IN_INTERNAL=1
        if eval "$__PEEK_COND" >/dev/null 2>&1; then
            __stop=1
        fi
        __PEEK_IN_INTERNAL=0
    fi

    if (( __stop == 0 )); then
        if (( __in_subshell == 0 )) && __peek_needs_tty "$__cmd"; then
            __peek_ensure_cmd_fd
            __peek_lock
            printf "TTY_HANDOFF\t%s\n" "${__cmd//$'\n'/ }" >&"$__PEEK_STATE_FD"
            local __ack
            IFS= read -r -u "$__PEEK_CMD_FD" __ack
            __peek_unlock
            if [[ "$__ack" == "TTY_GRANTED" ]]; then
                exec 0</dev/tty 1>/dev/tty 2>/dev/tty 2>/dev/null
                __PEEK_TTY_ACTIVE=1
            fi
        fi
        SECONDS=$__freeze_sec
        __peek_restore_env "$__flags" "$__had_pipefail" "$__had_nocase" "$__had_extglob" "$__had_nullglob"
        __peek_set_rc "$__rc"
        return 0
    fi

    __peek_ensure_cmd_fd
    __peek_lock
    __peek_emit_frame "STOP" "$__src" "$__line" "$__rc" "$__pipe_str" "$__cmd"

    local __force_tty=0
    local __msg
    while IFS= read -r -u "$__PEEK_CMD_FD" __msg; do
        case "$__msg" in
            RESUME:*)
                local __payload="${__msg#RESUME:}"
                IFS='|' read -r __PEEK_MODE __PEEK_TARGET_DEPTH __PEEK_ERR_TRAP __PEEK_BPS __PEEK_COND <<< "$__payload"
                if [[ -n "$__PEEK_SESSION_DIR" ]]; then
                    export PEEK_INIT_BPS="$__PEEK_BPS"
                    export PEEK_INIT_ERR_TRAP="$__PEEK_ERR_TRAP"
                    export PEEK_INIT_COND="$__PEEK_COND"
                fi
                break
                ;;
            RESUME_TTY:*)
                local __payload="${__msg#RESUME_TTY:}"
                IFS='|' read -r __PEEK_MODE __PEEK_TARGET_DEPTH __PEEK_ERR_TRAP __PEEK_BPS __PEEK_COND <<< "$__payload"
                if [[ -n "$__PEEK_SESSION_DIR" ]]; then
                    export PEEK_INIT_BPS="$__PEEK_BPS"
                    export PEEK_INIT_ERR_TRAP="$__PEEK_ERR_TRAP"
                    export PEEK_INIT_COND="$__PEEK_COND"
                fi
                __force_tty=1
                break
                ;;
            SKIP)
                __PEEK_MODE="STEP"
                __peek_unlock
                SECONDS=$__freeze_sec
                __peek_restore_env "$__flags" "$__had_pipefail" "$__had_nocase" "$__had_extglob" "$__had_nullglob"
                return 1
                ;;
            EVAL:*)
                local __expr="${__msg#EVAL:}"
                __PEEK_IN_INTERNAL=1
                if (( __had_nocase == 1 )); then builtin shopt -s nocasematch; fi
                if (( __had_extglob == 1 )); then builtin shopt -s extglob; fi
                eval "$__expr" >&1 2>&2
                builtin shopt -u nocasematch
                __PEEK_IN_INTERNAL=0
                __peek_emit_frame "SYNC" "$__src" "$__line" "$__rc" "$__pipe_str" "$__cmd"
                ;;
            ABORT)
                __peek_unlock
                exit 130
                ;;
        esac
    done

    if (( (__force_tty == 1 || __in_subshell == 0) )) && { (( __force_tty == 1 )) || __peek_needs_tty "$__cmd"; }; then
        printf "TTY_HANDOFF\t%s\n" "${__cmd//$'\n'/ }" >&"$__PEEK_STATE_FD"
        local __ack
        IFS= read -r -u "$__PEEK_CMD_FD" __ack
        if [[ "$__ack" == "TTY_GRANTED" ]]; then
            exec 0</dev/tty 1>/dev/tty 2>/dev/tty 2>/dev/null
            __PEEK_TTY_ACTIVE=1
        fi
    fi

    __peek_unlock
    SECONDS=$__freeze_sec
    __peek_restore_env "$__flags" "$__had_pipefail" "$__had_nocase" "$__had_extglob" "$__had_nullglob"
    __peek_set_rc "$__rc"
    return 0
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    BASH_ARGV0="$__PEEK_TARGET_SCRIPT"
    __peek_emit_proc_start "root" "$__PEEK_TARGET_SCRIPT"
    set -o functrace
    set -o errtrace
    builtin shopt -s extdebug
    builtin trap '__peek_on_exit' EXIT
    builtin trap '__peek_hook "$LINENO" "$BASH_COMMAND" "$?" "${PIPESTATUS[*]}"' DEBUG
    source "$__PEEK_TARGET_SCRIPT" "$@"
    __peek_final_rc=$?
    exit $__peek_final_rc
elif [[ "$-" != *c* && -f "$0" && "${__PEEK_WRAPPED_PID:-}" != "$BASHPID" ]]; then
    __PEEK_WRAPPED_PID="$BASHPID"
    __PEEK_ROOT_PID="$BASHPID"
    __PEEK_TARGET_SCRIPT="$0"
    __PEEK_MODE="${PEEK_CHILD_MODE:-CONT}"
    __PEEK_TARGET_DEPTH=0
    BASH_ARGV0="$__PEEK_TARGET_SCRIPT"
    __peek_emit_proc_start "child" "$__PEEK_TARGET_SCRIPT"
    set -o functrace
    set -o errtrace
    builtin shopt -s extdebug
    builtin trap '__peek_on_exit' EXIT
    builtin trap '__peek_hook "$LINENO" "$BASH_COMMAND" "$?" "${PIPESTATUS[*]}"' DEBUG
    source "$__PEEK_TARGET_SCRIPT" "$@"
    __peek_final_rc=$?
    exit $__peek_final_rc
fi
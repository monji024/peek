# https://github.com/monji024

exec {__PEEK_STATE_FD}> "$PEEK_STATE_FIFO"
exec {__PEEK_CMD_FD}< "$PEEK_CMD_FIFO"
exec {__PEEK_LOCK_FD}> "$PEEK_LOCK_FILE"
exec {__PEEK_SAVED_IN}<&0
exec {__PEEK_SAVED_OUT}>&1
exec {__PEEK_SAVED_ERR}>&2

__PEEK_ROOT_PID=$BASHPID
__PEEK_NONCE="$PEEK_NONCE"
__PEEK_BOOT="$PEEK_BOOTSTRAP"
__PEEK_TARGET_SCRIPT="$PEEK_TARGET"
__PEEK_MODE="STEP"
__PEEK_TARGET_DEPTH=0
__PEEK_ERR_TRAP="${PEEK_INIT_ERR_TRAP:-0}"
__PEEK_BPS="${PEEK_INIT_BPS:-}"
__PEEK_COND=""
__PEEK_SKIP_ACTIVE=0
__PEEK_TTY_ACTIVE=0
__PEEK_HAS_FLOCK=0
command -v flock >/dev/null 2>&1 && __PEEK_HAS_FLOCK=1

unset PEEK_STATE_FIFO PEEK_CMD_FIFO PEEK_LOCK_FILE PEEK_BOOTSTRAP PEEK_TARGET PEEK_INIT_BPS PEEK_INIT_ERR_TRAP PEEK_NONCE

__peek_lock() {
    (( __PEEK_HAS_FLOCK == 1 )) && flock -x "$__PEEK_LOCK_FD" 2>/dev/null
}

__peek_unlock() {
    (( __PEEK_HAS_FLOCK == 1 )) && flock -u "$__PEEK_LOCK_FD" 2>/dev/null
}

__peek_needs_tty() {
    local __c="$1"
    [[ "$__c" =~ (^|[[:space:]|;&\(])(sudo[[:space:]]+)?(vim|nvim|vi|nano|emacs|fzf|sk|dialog|whiptail|gum|less|more|man|htop|top|btop|lazygit|tig|ssh)([[:space:]|;&\)]|$) ]]
}

__peek_emit_frame() {
    local __kind="$1"
    local __src="$2"
    local __line="$3"
    local __rc="$4"
    local __pipe_str="$5"
    local __cmd="${6//$'\n'/ }"
    local __pwd="${PWD//$'\n'/ }"

    printf "%s\t%s\t%s\t%s\t%s\t%s\n" \
        "$__kind" "$__src" "$__line" "$__rc" "$BASH_SUBSHELL" "$__pipe_str" >&"$__PEEK_STATE_FD"
    printf "PWD:%s\n" "$__pwd" >&"$__PEEK_STATE_FD"
    printf "CMD:%s\n" "$__cmd" >&"$__PEEK_STATE_FD"

    local __idx
    for (( __idx=1; __idx < ${#FUNCNAME[@]}; __idx++ )); do
        local __fn="${FUNCNAME[$__idx]}"
        local __fsrc="${BASH_SOURCE[$__idx]:-$__src}"
        local __flin="${BASH_LINENO[$((__idx-1))]:-0}"
        if [[ "$__fn" != "source" && "$__fn" != "." && "$__fsrc" != "$__PEEK_BOOT" ]]; then
            printf "FRAME:%s\t%s\t%s\n" "$__fn" "$__fsrc" "$__flin" >&"$__PEEK_STATE_FD"
        fi
    done

    printf "__PEEK_VARS_%s__\n" "$__PEEK_NONCE" >&"$__PEEK_STATE_FD"
    declare -p >&"$__PEEK_STATE_FD" 2>/dev/null
    printf "\n__PEEK_END_%s__\n" "$__PEEK_NONCE" >&"$__PEEK_STATE_FD"
}

__peek_restore_env() {
    local __had_e="$1"
    local __had_u="$2"
    local __had_nocase="$3"
    (( __had_nocase == 1 )) && shopt -s nocasematch
    (( __had_e == 1 )) && set -e
    (( __had_u == 1 )) && set -u
}

__peek_reclaim_tty() {
    if (( __PEEK_TTY_ACTIVE == 1 )); then
        exec 0<&"$__PEEK_SAVED_IN" 1>&"$__PEEK_SAVED_OUT" 2>&"$__PEEK_SAVED_ERR"
        __PEEK_TTY_ACTIVE=0
        printf "TTY_DONE\t%s\n" "$__PEEK_NONCE" >&"$__PEEK_STATE_FD"
    fi
}

__peek_set_rc() {
    return "$1"
}

__peek_hook() {
    local __rc=$?
    local -a __pipe_arr=("${PIPESTATUS[@]}")
    local __freeze_sec=$SECONDS
    local __had_e=0
    local __had_u=0
    local __had_nocase=0

    [[ -o errexit ]] && __had_e=1
    [[ -o nounset ]] && __had_u=1
    shopt -q nocasematch && __had_nocase=1

    set +eu
    shopt -u nocasematch

    __peek_reclaim_tty

    if (( __PEEK_SKIP_ACTIVE == 1 )); then
        shopt -u extdebug
        __PEEK_SKIP_ACTIVE=0
    fi

    local __src="${BASH_SOURCE[1]}"
    if [[ -z "$__src" || "$__src" == "$__PEEK_BOOT" ]]; then
        SECONDS=$__freeze_sec
        __peek_restore_env "$__had_e" "$__had_u" "$__had_nocase"
        __peek_set_rc "$__rc"
        return 0
    fi

    local __pipe_str="${__pipe_arr[*]}"
    local __line="$1"
    local __cmd="$2"
    local __depth=$(( ${#FUNCNAME[@]} - 2 ))
    (( __depth < 0 )) && __depth=0

    local __in_subshell=0
    (( BASHPID != __PEEK_ROOT_PID )) && __in_subshell=1

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
        if eval "$__PEEK_COND" >/dev/null 2>&1; then
            __stop=1
        fi
    fi

    if (( __stop == 0 )); then
        if (( __in_subshell == 0 )) && __peek_needs_tty "$__cmd"; then
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
        __peek_restore_env "$__had_e" "$__had_u" "$__had_nocase"
        __peek_set_rc "$__rc"
        return 0
    fi

    __peek_lock
    __peek_emit_frame "STOP" "$__src" "$__line" "$__rc" "$__pipe_str" "$__cmd"

    local __force_tty=0
    local __msg
    while IFS= read -r -u "$__PEEK_CMD_FD" __msg; do
        case "$__msg" in
            RESUME:*)
                local __payload="${__msg#RESUME:}"
                IFS='|' read -r __PEEK_MODE __PEEK_TARGET_DEPTH __PEEK_ERR_TRAP __PEEK_BPS __PEEK_COND <<< "$__payload"
                break
                ;;
            RESUME_TTY:*)
                local __payload="${__msg#RESUME_TTY:}"
                IFS='|' read -r __PEEK_MODE __PEEK_TARGET_DEPTH __PEEK_ERR_TRAP __PEEK_BPS __PEEK_COND <<< "$__payload"
                __force_tty=1
                break
                ;;
            SKIP)
                __PEEK_MODE="STEP"
                __PEEK_SKIP_ACTIVE=1
                __peek_unlock
                shopt -s extdebug
                SECONDS=$__freeze_sec
                __peek_restore_env "$__had_e" "$__had_u" "$__had_nocase"
                return 1
                ;;
            EVAL:*)
                local __expr="${__msg#EVAL:}"
                (( __had_nocase == 1 )) && shopt -s nocasematch
                eval "$__expr" >&1 2>&2
                shopt -u nocasematch
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
    __peek_restore_env "$__had_e" "$__had_u" "$__had_nocase"
    __peek_set_rc "$__rc"
    return 0
}

BASH_ARGV0="$__PEEK_TARGET_SCRIPT"
set -o functrace
set -o errtrace
trap '__peek_hook "$LINENO" "$BASH_COMMAND"' DEBUG
source "$__PEEK_TARGET_SCRIPT" "$@"
__peek_final_rc=$?
__peek_reclaim_tty
exit $__peek_final_rc
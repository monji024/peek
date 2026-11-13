#!/usr/bin/env python3
# https://github.com/monji024

import os
import select
import subprocess
import sys
import tempfile

WORK_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BOOTSTRAP_SRC = os.path.join(WORK_DIR, "src/engine/bootstrap.sh")


def run_debug_session(
    script_content,
    actions,
    args=None,
    break_on_err=False,
    bps="",
    cond="",
    extra_files=None,
    stdin_lines=None,
):
    with tempfile.TemporaryDirectory(dir=WORK_DIR) as tmp:
        state_fifo = os.path.join(tmp, "state.fifo")
        cmd_fifo = os.path.join(tmp, "cmd.fifo")
        lock_file = os.path.join(tmp, "subshell.lock")
        boot_path = os.path.join(tmp, "bootstrap.sh")
        script_path = os.path.join(tmp, "target.sh")

        os.mkfifo(state_fifo, 0o600)
        os.mkfifo(cmd_fifo, 0o600)
        open(lock_file, "w").close()
        with open(BOOTSTRAP_SRC, "r", encoding="utf-8") as f:
            with open(boot_path, "w", encoding="utf-8") as out:
                out.write(f.read())

        if extra_files:
            for name, content in extra_files.items():
                cpath = os.path.join(tmp, name)
                with open(cpath, "w", encoding="utf-8") as cf:
                    cf.write(content.replace("__TMP__", tmp))
                os.chmod(cpath, 0o755)

        with open(script_path, "w", encoding="utf-8") as f:
            f.write(script_content.replace("__TMP__", tmp))
        os.chmod(script_path, 0o755)

        state_fd = os.open(state_fifo, os.O_RDWR | os.O_NONBLOCK)
        cmd_fd = os.open(cmd_fifo, os.O_RDWR)

        nonce = "a1b2c3d4e5f60718"
        resolved_bps = (
            bps.replace("__SCRIPT__", script_path).replace("__TMP__", tmp)
        )
        env = os.environ.copy()
        env.update(
            {
                "PEEK_STATE_FIFO": state_fifo,
                "PEEK_CMD_FIFO": cmd_fifo,
                "PEEK_LOCK_FILE": lock_file,
                "PEEK_BOOTSTRAP": boot_path,
                "PEEK_TARGET": script_path,
                "PEEK_SESSION_DIR": tmp,
                "PEEK_INIT_BPS": resolved_bps,
                "PEEK_INIT_ERR_TRAP": "1" if break_on_err else "0",
                "PEEK_INIT_COND": cond,
                "PEEK_NONCE": nonce,
            }
        )

        cmd = ["bash", boot_path] + (args or [])
        proc = subprocess.Popen(
            cmd,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        if stdin_lines:
            for sline in stdin_lines:
                proc.stdin.write(sline + "\n")
            proc.stdin.flush()

        buf = b""
        frames = []
        events = []
        action_idx = 0
        end_marker = f"__PEEK_END_{nonce}__\n".encode()

        while True:
            r, _, _ = select.select([state_fd], [], [], 3.0)
            if not r:
                if proc.poll() is not None:
                    break
                proc.kill()
                raise RuntimeError(
                    f"Timeout waiting for state_fd at action_idx={action_idx}"
                )
            chunk = os.read(state_fd, 65536)
            if not chunk:
                break
            buf += chunk

            while True:
                nl = buf.find(b"\n")
                if nl == -1:
                    break
                first_line = buf[:nl].decode("utf-8", "replace")
                if (
                    first_line.startswith("PROC_START\t")
                    or first_line.startswith("PROC_EXIT\t")
                    or first_line.startswith("TTY_DONE\t")
                ):
                    events.append(first_line)
                    buf = buf[nl + 1 :]
                    continue
                elif first_line.startswith("TTY_HANDOFF\t"):
                    events.append(first_line)
                    buf = buf[nl + 1 :]
                    os.write(cmd_fd, b"TTY_GRANTED\n")
                    continue
                elif first_line.startswith("STOP\t") or first_line.startswith(
                    "SYNC\t"
                ):
                    end_pos = buf.find(end_marker)
                    if end_pos == -1:
                        break
                    frame_bytes = buf[:end_pos]
                    buf = buf[end_pos + len(end_marker) :]
                    lines = frame_bytes.decode("utf-8", "replace").splitlines()
                    hdr = lines[0].split("\t")
                    pwd_line = next(
                        (l[4:] for l in lines if l.startswith("PWD:")), ""
                    )
                    cmd_line = next(
                        (l[4:] for l in lines if l.startswith("CMD:")), ""
                    )
                    stack_lines = [
                        l[6:] for l in lines if l.startswith("FRAME:")
                    ]
                    var_lines = []
                    in_v = False
                    for l in lines:
                        if l == f"__PEEK_VARS_{nonce}__":
                            in_v = True
                            continue
                        if in_v:
                            var_lines.append(l)
                    frames.append(
                        {
                            "kind": hdr[0],
                            "file": os.path.basename(hdr[1]),
                            "full_file": hdr[1],
                            "line": int(hdr[2]),
                            "rc": int(hdr[3]),
                            "subshell": int(hdr[4]),
                            "pipe": hdr[5],
                            "pid": int(hdr[6]),
                            "ppid": int(hdr[7]),
                            "pwd": pwd_line,
                            "cmd": cmd_line,
                            "stack": stack_lines,
                            "vars": var_lines,
                            "tmp": tmp,
                            "script_path": script_path,
                        }
                    )
                    if action_idx < len(actions):
                        act = actions[action_idx](frames[-1])
                        action_idx += 1
                    else:
                        act = f"RESUME:CONT|0|{1 if break_on_err else 0}|{resolved_bps}|{cond}\n"
                    os.write(cmd_fd, act.encode())
                else:
                    buf = buf[nl + 1 :]

            if proc.poll() is not None and not buf:
                break

        out, err = proc.communicate(timeout=3)
        os.close(state_fd)
        os.close(cmd_fd)
        return proc.returncode, out, err, frames, events


def test_simple_script_and_args():
    rc, out, err, frames, events = run_debug_session(
        """#!/usr/bin/env bash
A="$1"
B="$2"
echo "script=$0 A=$A B=$B"
""",
        [
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
        ],
        args=["alpha", "beta"],
    )
    assert rc == 0, f"rc={rc} err={err}"
    assert [f["line"] for f in frames] == [2, 3, 4]
    assert "A=alpha B=beta" in out
    assert "target.sh" in out


def test_functions_and_nested_calls():
    rc, out, err, frames, _ = run_debug_session(
        """#!/usr/bin/env bash
inner() {
    local z=99
    echo "inner:$z"
}
outer() {
    local y=42
    inner
    echo "outer:$y"
}
outer
echo "done"
""",
        [
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: f"RESUME:OUT|{len(f['stack'])}|0||\n",
            lambda f: "RESUME:CONT|0|0||\n",
        ],
    )
    assert rc == 0
    assert [f["line"] for f in frames] == [11, 6, 7, 8, 2, 3, 9], f"lines={[f['line'] for f in frames]}"
    assert len(frames[5]["stack"]) == 2


def test_pipelines_and_pipestatus():
    rc, out, err, frames, _ = run_debug_session(
        """#!/usr/bin/env bash
false | true | true
echo "ps=${PIPESTATUS[*]}"
""",
        [
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
        ],
    )
    assert rc == 0
    assert "ps=1 0 0" in out, f"out={out!r}"
    assert frames[3]["pipe"] == "1 0 0", f"frame pipe={frames[3]['pipe']!r}"


def test_subshells_and_command_substitutions():
    rc, out, err, frames, _ = run_debug_session(
        """#!/usr/bin/env bash
VAL=$(echo "sub_a"; echo "sub_b")
echo "VAL=$VAL"
""",
        [
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:CONT|0|0||\n",
        ],
    )
    assert rc == 0
    assert any(f["subshell"] >= 1 for f in frames), "Did not step into subshell"
    assert "VAL=sub_a" in out


def test_sourced_scripts():
    rc, out, err, frames, _ = run_debug_session(
        """#!/usr/bin/env bash
X=1
source "__TMP__/lib.sh"
echo "X=$X LIB=$LIB_VAR"
""",
        [
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:CONT|0|0||\n",
        ],
        extra_files={
            "lib.sh": """LIB_VAR="from_lib"
"""
        },
    )
    assert rc == 0
    assert any(f["file"] == "lib.sh" for f in frames)
    assert "X=1 LIB=from_lib" in out


def test_arrays_and_associative_arrays():
    rc, out, err, frames, _ = run_debug_session(
        """#!/usr/bin/env bash
declare -a ARR=("first" $'second\\nline')
declare -A MAP=([k1]="val1" [k2]="val2")
echo "${ARR[0]} ${MAP[k1]}"
""",
        [
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:CONT|0|0||\n",
        ],
    )
    assert rc == 0
    v_str = "\n".join(frames[2]["vars"])
    assert "declare -a ARR=" in v_str
    assert "declare -A MAP=" in v_str


def test_conditional_expressions_and_short_circuit():
    rc, out, err, frames, _ = run_debug_session(
        """#!/usr/bin/env bash
for i in 1 2 3 4 5; do
    X=$(( i * 10 ))
done
echo "final X=$X"
""",
        [
            lambda f: "RESUME:CONT|0|0||(( i == 4 ))\n",
            lambda f: "RESUME:CONT|0|0||\n",
        ],
    )
    assert rc == 0
    assert len(frames) == 2
    assert frames[1]["line"] == 3
    assert any('declare -- i="4"' in v for v in frames[1]["vars"])


def test_shell_options_preservation():
    rc, out, err, frames, _ = run_debug_session(
        """#!/usr/bin/env bash
set -euo pipefail -f
shopt -s nocasematch nullglob
[[ "HELLO" == "hello" ]] && echo "nocase=ok"
[[ -o pipefail ]] && echo "pipefail=ok"
[[ "$-" == *f* ]] && echo "noglob=ok"
[[ "$-" == *e* ]] && echo "errexit=ok"
[[ "$-" == *u* ]] && echo "nounset=ok"
""",
        [
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:STEP|0|0||\n",
            lambda f: "RESUME:CONT|0|0||\n",
        ],
    )
    assert rc == 0
    for expected in ["nocase=ok", "pipefail=ok", "noglob=ok", "errexit=ok", "nounset=ok"]:
        assert expected in out, f"Missing {expected} in {out!r}"


def test_user_traps_and_signals():
    rc, out, err, frames, _ = run_debug_session(
        """#!/usr/bin/env bash
trap 'echo "USER_DEBUG_FIRED"' DEBUG
trap 'echo "USER_EXIT_FIRED"' EXIT
trap 'echo "USER_USR1_FIRED"' USR1
kill -USR1 $$
echo "body_done"
""",
        [lambda f: "RESUME:CONT|0|0||\n"],
    )
    assert rc == 0
    assert "USER_DEBUG_FIRED" in out
    assert "USER_EXIT_FIRED" in out
    assert "USER_USR1_FIRED" in out
    assert "body_done" in out


def test_background_jobs_and_exit_statuses():
    rc, out, err, frames, _ = run_debug_session(
        """#!/usr/bin/env bash
( sleep 0.05; echo "bg_done" ) &
BG_PID=$!
wait "$BG_PID"
false || true
exit 7
""",
        [lambda f: "RESUME:CONT|0|0||\n"],
    )
    assert rc == 7
    assert "bg_done" in out


def test_interactive_stdin_and_eval():
    rc, out, err, frames, _ = run_debug_session(
        """#!/usr/bin/env bash
read -r USER_IN
echo "read=$USER_IN injected=$INJECTED"
""",
        [
            lambda f: "EVAL:INJECTED=from_eval\n",
            lambda f: "RESUME:CONT|0|0||\n",
        ],
        stdin_lines=["hello_stdin"],
    )
    assert rc == 0
    assert frames[1]["kind"] == "SYNC"
    assert "read=hello_stdin injected=from_eval" in out


def test_tty_handoff_and_skip():
    rc, out, err, frames, events = run_debug_session(
        """#!/usr/bin/env bash
X="keep"
X="skip_this"
echo "X=$X"
""",
        [
            lambda f: "RESUME_TTY:STEP|0|0||\n",
            lambda f: "SKIP\n",
            lambda f: "RESUME:CONT|0|0||\n",
        ],
    )
    assert rc == 0
    assert out.strip() == "X=keep"
    assert any(e.startswith("TTY_HANDOFF\t") for e in events)
    assert any(e.startswith("TTY_DONE\t") for e in events)


def test_breakpoints_and_error_trap():
    rc, out, err, frames, _ = run_debug_session(
        """#!/usr/bin/env bash
A=1
false
B=2
C=3
echo "done $A $B $C"
""",
        [
            lambda f: f"RESUME:CONT|0|1|{f['script_path']}:5|\n",
            lambda f: f"RESUME:CONT|0|0|{f['script_path']}:5|\n",
            lambda f: "RESUME:CONT|0|0||\n",
        ],
    )
    assert rc == 0
    assert [f["line"] for f in frames] == [2, 4, 5], f"got {[f['line'] for f in frames]}"
    assert frames[1]["rc"] == 1


def test_multiple_concurrent_processes():
    rc, out, err, frames, events = run_debug_session(
        """#!/usr/bin/env bash
MAIN_STEP=1
"__TMP__/worker.sh"
MAIN_STEP=2
""",
        [
            lambda f: f"RESUME:CONT|0|0|{f['tmp']}/worker.sh:3|\n",
            lambda f: "RESUME:CONT|0|0||\n",
        ],
        extra_files={
            "worker.sh": """#!/usr/bin/env bash
W=10
W=20
echo "worker W=$W"
"""
        },
    )
    assert rc == 0
    assert any(f["file"] == "worker.sh" and f["line"] == 3 for f in frames)
    pids = {f["pid"] for f in frames}
    assert len(pids) == 2, f"Expected 2 distinct PIDs, got {pids}"


def test_session_disconnect_and_abort():
    rc, out, err, frames, _ = run_debug_session(
        """#!/usr/bin/env bash
A=1
echo "should not reach"
""",
        [lambda f: "ABORT\n"],
    )
    assert rc == 130
    assert "should not reach" not in out


def main():
    tests = [
        ("Simple scripts & arguments", test_simple_script_and_args),
        ("Functions, nested calls & step out", test_functions_and_nested_calls),
        ("Pipelines & PIPESTATUS", test_pipelines_and_pipestatus),
        ("Subshells & command substitutions", test_subshells_and_command_substitutions),
        ("Sourced scripts", test_sourced_scripts),
        ("Arrays & associative arrays", test_arrays_and_associative_arrays),
        ("Conditional expressions & loops", test_conditional_expressions_and_short_circuit),
        ("Shell options preservation", test_shell_options_preservation),
        ("User traps & signals", test_user_traps_and_signals),
        ("Background jobs & exit statuses", test_background_jobs_and_exit_statuses),
        ("Interactive stdin & live eval", test_interactive_stdin_and_eval),
        ("TTY handoff & extdebug command skip", test_tty_handoff_and_skip),
        ("Breakpoints & error traps", test_breakpoints_and_error_trap),
        ("Multiple concurrent Bash processes", test_multiple_concurrent_processes),
        ("Session abort & termination", test_session_disconnect_and_abort),
    ]

    for label, fn in tests:
        fn()
        print(f"  [PASS] {label}")
    print(f"\nAll {len(tests)} Bash integration test suites passed!")


if __name__ == "__main__":
    main()

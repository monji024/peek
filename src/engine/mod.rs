// https://github.com/monji024

pub mod parser;

use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, ErrorKind, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tempfile::TempDir;

use self::parser::{parse_frame, read_lossy_line, RawFrame};

const BOOTSTRAP_SCRIPT: &str = include_str!("bootstrap.sh");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeMode {
    Step,
    Next,
    Out,
    Continue,
}

impl ResumeMode {
    fn as_protocol_str(self) -> &'static str {
        match self {
            ResumeMode::Step => "STEP",
            ResumeMode::Next => "NEXT",
            ResumeMode::Out => "OUT",
            ResumeMode::Continue => "CONT",
        }
    }
}

pub enum EngineEvent {
    Frame(RawFrame),
    TtyHandoff(String),
    TtyDone,
    Stdout(String),
    Stderr(String),
}

pub struct Session {
    _temp_dir: TempDir,
    child: Child,
    child_stdin: Option<ChildStdin>,
    cmd_fifo: File,
    rx: Receiver<EngineEvent>,
}

impl Session {
    pub fn spawn(
        target_script: &Path,
        args: &[String],
        initial_bps: &[(PathBuf, usize)],
        break_on_err: bool,
    ) -> Result<Self> {
        let temp_dir = tempfile::tempdir().context("failed to allocate session tempdir")?;
        let state_path = temp_dir.path().join("state.fifo");
        let cmd_path = temp_dir.path().join("cmd.fifo");
        let lock_path = temp_dir.path().join("subshell.lock");
        let bootstrap_path = temp_dir.path().join("bootstrap.sh");

        create_fifo(&state_path)?;
        create_fifo(&cmd_path)?;
        File::create(&lock_path)?;
        std::fs::write(&bootstrap_path, BOOTSTRAP_SCRIPT)?;

        let state_fifo = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&state_path)
            .context("failed to open state FIFO")?;

        let cmd_fifo = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&cmd_path)
            .context("failed to open command FIFO")?;

        tune_pipe_capacity(state_fifo.as_raw_fd());

        let nonce = generate_session_nonce();
        let bp_spec = format_bp_list(initial_bps);

        let mut cmd = Command::new("bash");
        cmd.arg(&bootstrap_path)
            .args(args)
            .env("PEEK_STATE_FIFO", &state_path)
            .env("PEEK_CMD_FIFO", &cmd_path)
            .env("PEEK_LOCK_FILE", &lock_path)
            .env("PEEK_BOOTSTRAP", &bootstrap_path)
            .env("PEEK_TARGET", target_script)
            .env("PEEK_INIT_BPS", bp_spec)
            .env("PEEK_INIT_ERR_TRAP", if break_on_err { "1" } else { "0" })
            .env("PEEK_NONCE", &nonce)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);

        let mut child = cmd.spawn().context("failed to spawn bash process")?;
        let child_stdin = child.stdin.take();

        let (tx, rx) = mpsc::channel();

        if let Some(stdout) = child.stdout.take() {
            let tx_out = tx.clone();
            thread::spawn(move || pump_output(stdout, tx_out, false));
        }

        if let Some(stderr) = child.stderr.take() {
            let tx_err = tx.clone();
            thread::spawn(move || pump_output(stderr, tx_err, true));
        }

        thread::spawn(move || {
            let mut reader = BufReader::new(state_fifo);
            let mut raw_buf = Vec::with_capacity(512);
            let tty_done_marker = format!("TTY_DONE\t{}", nonce);

            while let Ok(Some(header)) = read_lossy_line(&mut reader, &mut raw_buf) {
                if header.starts_with("STOP\t") || header.starts_with("SYNC\t") {
                    if let Ok(Some(frame)) = parse_frame(&header, &mut reader, &nonce) {
                        if tx.send(EngineEvent::Frame(frame)).is_err() {
                            break;
                        }
                    }
                } else if let Some(cmd_str) = header.strip_prefix("TTY_HANDOFF\t") {
                    if tx.send(EngineEvent::TtyHandoff(cmd_str.to_string())).is_err() {
                        break;
                    }
                } else if header == tty_done_marker {
                    if tx.send(EngineEvent::TtyDone).is_err() {
                        break;
                    }
                }
            }
        });

        Ok(Self {
            _temp_dir: temp_dir,
            child,
            child_stdin,
            cmd_fifo,
            rx,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn try_recv(&self) -> Option<EngineEvent> {
        self.rx.try_recv().ok()
    }

    pub fn try_wait_exit(&mut self) -> Result<Option<i32>> {
        if let Some(status) = self.child.try_wait()? {
            Ok(Some(status.code().unwrap_or(-1)))
        } else {
            Ok(None)
        }
    }

    pub fn send_resume(
        &mut self,
        mode: ResumeMode,
        target_depth: usize,
        break_on_err: bool,
        bps: &[(PathBuf, usize)],
        cond_expr: &str,
        force_tty: bool,
    ) -> Result<()> {
        let bp_spec = format_bp_list(bps);
        let clean_cond = cond_expr.replace(['\n', '|'], " ");
        let verb = if force_tty { "RESUME_TTY" } else { "RESUME" };
        let msg = format!(
            "{}:{}|{}|{}|{}|{}\n",
            verb,
            mode.as_protocol_str(),
            target_depth,
            if break_on_err { 1 } else { 0 },
            bp_spec,
            clean_cond
        );
        self.write_cmd(msg.as_bytes())
    }

    pub fn send_tty_granted(&mut self) -> Result<()> {
        self.write_cmd(b"TTY_GRANTED\n")
    }

    pub fn send_skip(&mut self) -> Result<()> {
        self.write_cmd(b"SKIP\n")
    }

    pub fn send_eval(&mut self, expr: &str) -> Result<()> {
        let sanitized = expr.replace('\n', "; ");
        let msg = format!("EVAL:{}\n", sanitized);
        self.write_cmd(msg.as_bytes())
    }

    pub fn send_stdin(&mut self, input: &str) -> Result<()> {
        if let Some(stdin) = self.child_stdin.as_mut() {
            let mut payload = input.as_bytes().to_vec();
            payload.push(b'\n');
            let _ = stdin.write_all(&payload);
            let _ = stdin.flush();
        }
        Ok(())
    }

    pub fn shutdown(&mut self) {
        let _ = self.write_cmd(b"ABORT\n");
        let pgid = self.child.id() as i32;
        if pgid > 0 {
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn write_cmd(&mut self, bytes: &[u8]) -> Result<()> {
        match self.cmd_fifo.write_all(bytes) {
            Ok(_) => {
                let _ = self.cmd_fifo.flush();
                Ok(())
            }
            Err(e) if e.kind() == ErrorKind::BrokenPipe => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn create_fifo(path: &Path) -> Result<()> {
    let c_path = CString::new(path.as_os_str().as_bytes())?;
    let rc = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error()).context("mkfifo syscall failed");
    }
    Ok(())
}

fn tune_pipe_capacity(fd: i32) {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::fcntl(fd, libc::F_SETPIPE_SZ, 1_048_576);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = fd;
}

fn generate_session_nonce() -> String {
    let mut bytes = [0u8; 8];
    if let Ok(mut f) = File::open("/dev/urandom") {
        if f.read_exact(&mut bytes).is_ok() {
            return format!("{:016x}", u64::from_ne_bytes(bytes));
        }
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e3779b97f4a7c15);
    format!("{:016x}", nanos ^ (std::process::id() as u64))
}

fn format_bp_list(bps: &[(PathBuf, usize)]) -> String {
    bps.iter()
        .map(|(file, line)| format!("{}:{}", file.display(), line))
        .collect::<Vec<_>>()
        .join(" ")
}

fn pump_output<R: Read>(stream: R, tx: mpsc::Sender<EngineEvent>, is_stderr: bool) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::with_capacity(256);
    while let Ok(Some(line)) = read_lossy_line(&mut reader, &mut buf) {
        let ev = if is_stderr {
            EngineEvent::Stderr(line)
        } else {
            EngineEvent::Stdout(line)
        }
        ;
        if tx.send(ev).is_err() {
            break;
        }
    }
}
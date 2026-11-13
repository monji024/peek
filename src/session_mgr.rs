// https://github.com/monji024

use std::collections::BTreeMap;
use std::fs::{self, Permissions};
use std::io::BufReader;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use crate::engine::{parser::RawFrame, EngineEvent, Session};
use crate::protocol::{
    read_bounded_message, write_message, ClientMessage, DebugEvent, EventKind, ProcessMeta,
    ProcessStatus, ServerMessage, PROTOCOL_VERSION,
};

pub struct SessionSummary {
    pub session_id: String,
    pub socket_path: PathBuf,
    pub root_pid: u32,
    pub entry_script: String,
}

pub fn user_session_root() -> Result<PathBuf> {
    let uid = unsafe { libc::geteuid() };
    let base = std::env::temp_dir().join(format!("peek-sessions-{}", uid));
    if !base.exists() {
        fs::create_dir_all(&base)
            .with_context(|| format!("failed to create session directory: {:?}", base))?;
        fs::set_permissions(&base, Permissions::from_mode(0o700))?;
    } else {
        let meta = fs::symlink_metadata(&base)?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            bail!("session root {:?} is not a secure directory", base);
        }
        let _ = fs::set_permissions(&base, Permissions::from_mode(0o700));
    }
    Ok(base)
}

pub fn cleanup_stale_sessions() -> Result<usize> {
    let root = user_session_root()?;
    let mut removed = 0usize;
    let Ok(rd) = fs::read_dir(&root) else {
        return Ok(0);
    };

    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("meta") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let mut pid: u32 = 0;
        let mut sock_path: Option<PathBuf> = None;
        for line in content.lines() {
            if let Some(p) = line.strip_prefix("PID=") {
                pid = p.parse().unwrap_or(0);
            } else if let Some(s) = line.strip_prefix("SOCK=") {
                sock_path = Some(PathBuf::from(s));
            }
        }
        let alive = pid > 0 && unsafe { libc::kill(pid as i32, 0) == 0 };
        if !alive {
            let _ = fs::remove_file(&path);
            if let Some(sp) = sock_path {
                let _ = fs::remove_file(sp);
            }
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn list_active_sessions() -> Result<Vec<SessionSummary>> {
    let _ = cleanup_stale_sessions();
    let root = user_session_root()?;
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(&root) else {
        return Ok(out);
    };

    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("meta") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let mut id = String::new();
        let mut pid = 0u32;
        let mut sock = PathBuf::new();
        let mut script = String::new();
        for line in content.lines() {
            if let Some(v) = line.strip_prefix("ID=") {
                id = v.to_string();
            } else if let Some(v) = line.strip_prefix("PID=") {
                pid = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("SOCK=") {
                sock = PathBuf::from(v);
            } else if let Some(v) = line.strip_prefix("SCRIPT=") {
                script = v.to_string();
            }
        }
        if !id.is_empty() && sock.exists() {
            out.push(SessionSummary {
                session_id: id,
                socket_path: sock,
                root_pid: pid,
                entry_script: script,
            });
        }
    }
    out.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    Ok(out)
}

pub fn resolve_session_socket(id_or_prefix: &str) -> Result<PathBuf> {
    let active = list_active_sessions()?;
    if id_or_prefix.is_empty() || id_or_prefix == "latest" {
        if let Some(last) = active.last() {
            return Ok(last.socket_path.clone());
        }
        bail!("no active peek debugging sessions found");
    }
    for s in &active {
        if s.session_id == id_or_prefix || s.root_pid.to_string() == id_or_prefix {
            return Ok(s.socket_path.clone());
        }
    }
    for s in &active {
        if s.session_id.starts_with(id_or_prefix) || s.entry_script.contains(id_or_prefix) {
            return Ok(s.socket_path.clone());
        }
    }
    bail!(
        "session '{}' not found (run `peek --list-sessions` to view active sessions)",
        id_or_prefix
    )
}

pub fn run_headless_server(
    session_name: Option<String>,
    entry_script: PathBuf,
    args: Vec<String>,
    initial_bp_lines: &[usize],
    break_on_err: bool,
) -> Result<i32> {
    let _ = cleanup_stale_sessions();
    let root = user_session_root()?;

    let mut bps = Vec::new();
    for &line in initial_bp_lines {
        if line >= 1 {
            bps.push((entry_script.clone(), line));
        }
    }

    let mut session = Session::spawn(&entry_script, &args, &bps, break_on_err)?;
    let root_pid = session.pid();
    let session_id = session_name.unwrap_or_else(|| {
        let stem = entry_script
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("bash");
        format!("{}-{}", stem, root_pid)
    });

    let sock_path = root.join(format!("{}.sock", session_id));
    let meta_path = root.join(format!("{}.meta", session_id));
    if sock_path.exists() {
        let _ = fs::remove_file(&sock_path);
    }

    let listener = UnixListener::bind(&sock_path)
        .with_context(|| format!("failed to bind session socket: {:?}", sock_path))?;
    fs::set_permissions(&sock_path, Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;

    let meta_content = format!(
        "ID={}\nPID={}\nSOCK={}\nSCRIPT={}\n",
        session_id,
        root_pid,
        sock_path.display(),
        entry_script.display()
    );
    fs::write(&meta_path, meta_content)?;
    fs::set_permissions(&meta_path, Permissions::from_mode(0o600))?;

    eprintln!(
        "[peek] session '{}' listening on {} (target PID {})",
        session_id,
        sock_path.display(),
        root_pid
    );
    eprintln!(
        "[peek] attach from another terminal with: peek --attach {}",
        session_id
    );

    let mut procs: BTreeMap<u32, ProcessMeta> = BTreeMap::new();
    procs.insert(
        root_pid,
        ProcessMeta {
            pid: root_pid,
            ppid: std::process::id(),
            role: "root".into(),
            script: entry_script.clone(),
            status: ProcessStatus::Running,
        },
    );

    let mut last_frame: Option<RawFrame> = None;
    let mut seq = 1u64;
    let mut client_stream: Option<UnixStream> = None;
    let mut client_rx: Option<Receiver<ClientMessage>> = None;
    let mut final_exit = 0i32;

    loop {
        if let Ok((stream, _)) = listener.accept() {
            let _ = stream.set_nonblocking(false);
            if let Ok(reader_clone) = stream.try_clone() {
                let mut writer = stream;
                let welcome = ServerMessage::Welcome {
                    version: PROTOCOL_VERSION,
                    session_id: session_id.clone(),
                    root_pid,
                    entry_script: entry_script.clone(),
                };
                let _ = write_message(&mut writer, &welcome.encode());

                let proc_list: Vec<ProcessMeta> = procs.values().cloned().collect();
                let _ = write_message(&mut writer, &ServerMessage::ProcessList(proc_list).encode());

                let attach_ev = make_event(
                    &mut seq,
                    &session_id,
                    root_pid,
                    EventKind::DebuggerAttached,
                    "TUI client attached over Unix domain socket",
                );
                let _ = write_message(&mut writer, &ServerMessage::Event(attach_ev).encode());

                if let Some(ref f) = last_frame {
                    let _ = write_message(&mut writer, &ServerMessage::Frame(f.clone()).encode());
                }

                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
                    let mut reader = BufReader::new(reader_clone);
                    let mut buf = Vec::with_capacity(512);
                    while let Ok(Some(line)) = read_bounded_message(&mut reader, &mut buf) {
                        if let Ok(msg) = ClientMessage::decode(&line) {
                            let is_disc = matches!(msg, ClientMessage::Disconnect);
                            if tx.send(msg).is_err() || is_disc {
                                break;
                            }
                        }
                    }
                    let _ = tx.send(ClientMessage::Disconnect);
                });

                client_stream = Some(writer);
                client_rx = Some(rx);
            }
        }

        while let Some(ev) = session.try_recv() {
            match ev {
                EngineEvent::Frame(frame) => {
                    let fpid = if frame.pid > 0 { frame.pid } else { root_pid };
                    procs
                        .entry(fpid)
                        .and_modify(|m| m.status = ProcessStatus::Paused)
                        .or_insert(ProcessMeta {
                            pid: fpid,
                            ppid: frame.ppid,
                            role: if fpid == root_pid {
                                "root".into()
                            } else {
                                "child".into()
                            },
                            script: frame.file.clone(),
                            status: ProcessStatus::Paused,
                        });
                    last_frame = Some(frame.clone());
                    if let Some(w) = client_stream.as_mut() {
                        let list: Vec<ProcessMeta> = procs.values().cloned().collect();
                        let _ = write_message(w, &ServerMessage::ProcessList(list).encode());
                        let pev = make_event(
                            &mut seq,
                            &session_id,
                            fpid,
                            EventKind::ProcessPaused,
                            format!("{}:{}", frame.file.display(), frame.line),
                        );
                        let _ = write_message(w, &ServerMessage::Event(pev).encode());
                        if write_message(w, &ServerMessage::Frame(frame).encode()).is_err() {
                            client_stream = None;
                            client_rx = None;
                        }
                    }
                }
                EngineEvent::ProcessStarted {
                    pid,
                    ppid,
                    role,
                    script,
                } => {
                    procs.insert(
                        pid,
                        ProcessMeta {
                            pid,
                            ppid,
                            role: role.clone(),
                            script: script.clone(),
                            status: ProcessStatus::Running,
                        },
                    );
                    if let Some(w) = client_stream.as_mut() {
                        let list: Vec<ProcessMeta> = procs.values().cloned().collect();
                        let _ = write_message(w, &ServerMessage::ProcessList(list).encode());
                        let sev = make_event(
                            &mut seq,
                            &session_id,
                            pid,
                            EventKind::ProcessStarted,
                            format!("{} ({})", script.display(), role),
                        );
                        let _ = write_message(w, &ServerMessage::Event(sev).encode());
                    }
                }
                EngineEvent::ProcessExited { pid, exit_code } => {
                    if let Some(m) = procs.get_mut(&pid) {
                        m.status = ProcessStatus::Exited(exit_code);
                    }
                    if let Some(w) = client_stream.as_mut() {
                        let list: Vec<ProcessMeta> = procs.values().cloned().collect();
                        let _ = write_message(w, &ServerMessage::ProcessList(list).encode());
                        let eev = make_event(
                            &mut seq,
                            &session_id,
                            pid,
                            EventKind::ProcessExited,
                            format!("exit code {}", exit_code),
                        );
                        let _ = write_message(w, &ServerMessage::Event(eev).encode());
                    }
                }
                EngineEvent::Stdout(line) => {
                    println!("{}", line);
                    if let Some(w) = client_stream.as_mut() {
                        let _ = write_message(w, &ServerMessage::Stdout(line).encode());
                    }
                }
                EngineEvent::Stderr(line) => {
                    eprintln!("{}", line);
                    if let Some(w) = client_stream.as_mut() {
                        let _ = write_message(w, &ServerMessage::Stderr(line).encode());
                    }
                }
                EngineEvent::TtyHandoff(_cmd) => {
                    let _ = session.send_tty_granted();
                }
                EngineEvent::TtyDone => {}
            }
        }

        let mut drop_client = false;
        let mut shutdown_requested = false;
        if let Some(rx) = client_rx.as_ref() {
            while let Ok(cmsg) = rx.try_recv() {
                match cmsg {
                    ClientMessage::Hello { .. } => {}
                    ClientMessage::Resume {
                        pid,
                        mode,
                        target_depth,
                        break_on_err,
                        bps,
                        cond_expr,
                        force_tty,
                    } => {
                        let target_pid = if pid > 0 { pid } else { root_pid };
                        if let Some(m) = procs.get_mut(&target_pid) {
                            m.status = ProcessStatus::Running;
                        }
                        let _ = session.send_resume(
                            mode,
                            target_depth,
                            break_on_err,
                            &bps,
                            &cond_expr,
                            force_tty,
                        );
                        if let Some(w) = client_stream.as_mut() {
                            let list: Vec<ProcessMeta> = procs.values().cloned().collect();
                            let _ = write_message(w, &ServerMessage::ProcessList(list).encode());
                            let rev = make_event(
                                &mut seq,
                                &session_id,
                                target_pid,
                                EventKind::ProcessResumed,
                                mode.as_protocol_str(),
                            );
                            let _ = write_message(w, &ServerMessage::Event(rev).encode());
                        }
                    }
                    ClientMessage::Skip { pid } => {
                        let target_pid = if pid > 0 { pid } else { root_pid };
                        if let Some(m) = procs.get_mut(&target_pid) {
                            m.status = ProcessStatus::Running;
                        }
                        let _ = session.send_skip();
                    }
                    ClientMessage::Eval { expr, .. } => {
                        let _ = session.send_eval(&expr);
                    }
                    ClientMessage::SendStdin { input } => {
                        let _ = session.send_stdin(&input);
                    }
                    ClientMessage::TtyGranted { .. } => {
                        let _ = session.send_tty_granted();
                    }
                    ClientMessage::TakeSnapshot { .. } | ClientMessage::SelectProcess { .. } => {}
                    ClientMessage::Disconnect => {
                        drop_client = true;
                    }
                    ClientMessage::Shutdown => {
                        shutdown_requested = true;
                    }
                }
            }
        }

        if drop_client {
            client_stream = None;
            client_rx = None;
        }

        if shutdown_requested {
            session.shutdown();
            break;
        }

        if let Some(code) = session.try_wait_exit()? {
            final_exit = code;
            if let Some(w) = client_stream.as_mut() {
                let _ = write_message(w, &ServerMessage::SessionExited { exit_code: code }.encode());
            }
            break;
        }

        thread::sleep(Duration::from_millis(10));
    }

    let _ = fs::remove_file(&sock_path);
    let _ = fs::remove_file(&meta_path);
    Ok(final_exit)
}

fn make_event(
    seq: &mut u64,
    session_id: &str,
    pid: u32,
    kind: EventKind,
    detail: impl Into<String>,
) -> DebugEvent {
    let ev = DebugEvent {
        seq: *seq,
        timestamp_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        session_id: session_id.to_string(),
        pid,
        kind,
        detail: detail.into(),
    };
    *seq += 1;
    ev
}

pub struct RemoteClient {
    pub session_id: String,
    pub root_pid: u32,
    pub entry_script: PathBuf,
    writer: UnixStream,
    rx: Receiver<ServerMessage>,
}

impl RemoteClient {
    pub fn connect(sock_path: &Path) -> Result<Self> {
        let mut stream = UnixStream::connect(sock_path)
            .with_context(|| format!("failed to connect to session socket {:?}", sock_path))?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;

        let hello = ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            client_pid: std::process::id(),
        };
        write_message(&mut stream, &hello.encode())?;

        let reader_clone = stream.try_clone()?;
        let mut reader = BufReader::new(reader_clone);
        let mut buf = Vec::with_capacity(512);

        let welcome_line = read_bounded_message(&mut reader, &mut buf)?
            .ok_or_else(|| anyhow::anyhow!("server closed connection before WELCOME"))?;

        let (session_id, root_pid, entry_script) = match ServerMessage::decode(&welcome_line) {
            Ok(ServerMessage::Welcome {
                version,
                session_id,
                root_pid,
                entry_script,
            }) => {
                if version != PROTOCOL_VERSION {
                    bail!(
                        "protocol version mismatch: client v{}, server v{}",
                        PROTOCOL_VERSION,
                        version
                    );
                }
                (session_id, root_pid, entry_script)
            }
            Ok(other) => bail!("expected WELCOME from server, got {:?}", other),
            Err(e) => bail!("invalid WELCOME handshake: {}", e),
        };

        let _ = stream.set_read_timeout(None);
        let (tx, rx): (Sender<ServerMessage>, Receiver<ServerMessage>) = mpsc::channel();
        thread::spawn(move || {
            let mut local_buf = Vec::with_capacity(1024);
            while let Ok(Some(line)) = read_bounded_message(&mut reader, &mut local_buf) {
                if let Ok(msg) = ServerMessage::decode(&line) {
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
            }
            let _ = tx.send(ServerMessage::Error {
                message: "Session server disconnected".into(),
            });
        });

        Ok(Self {
            session_id,
            root_pid,
            entry_script,
            writer: stream,
            rx,
        })
    }

    pub fn try_recv(&self) -> Option<ServerMessage> {
        self.rx.try_recv().ok()
    }

    pub fn send(&mut self, msg: &ClientMessage) -> Result<()> {
        write_message(&mut self.writer, &msg.encode())?;
        Ok(())
    }

    pub fn disconnect(&mut self) {
        let _ = write_message(&mut self.writer, &ClientMessage::Disconnect.encode());
    }

    pub fn shutdown_remote(&mut self) {
        let _ = write_message(&mut self.writer, &ClientMessage::Shutdown.encode());
    }
}

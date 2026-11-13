// https://github.com/monji024

use std::fmt::Write as _;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use crate::engine::{parser::RawFrame, ResumeMode};
use crate::state::{StackFrame, VarKind};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_IPC_LINE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessStatus {
    Paused,
    Running,
    Exited(i32),
    Disconnected,
}

impl ProcessStatus {
    pub fn as_tag(&self) -> &'static str {
        match self {
            ProcessStatus::Paused => "PAUSED",
            ProcessStatus::Running => "RUNNING",
            ProcessStatus::Exited(_) => "EXITED",
            ProcessStatus::Disconnected => "DISCONNECTED",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessMeta {
    pub pid: u32,
    pub ppid: u32,
    pub role: String,
    pub script: PathBuf,
    pub status: ProcessStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    ProcessStarted,
    ProcessPaused,
    ProcessResumed,
    ProcessExited,
    DebuggerAttached,
    DebuggerDisconnected,
    FileCreated,
    FileModified,
    FileDeleted,
    SnapshotCaptured,
    SnapshotCompared,
    CommFailure,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::ProcessStarted => "PROC_START",
            EventKind::ProcessPaused => "PROC_PAUSE",
            EventKind::ProcessResumed => "PROC_RESUME",
            EventKind::ProcessExited => "PROC_EXIT",
            EventKind::DebuggerAttached => "DBG_ATTACH",
            EventKind::DebuggerDisconnected => "DBG_DETACH",
            EventKind::FileCreated => "FILE_CREATE",
            EventKind::FileModified => "FILE_MODIFY",
            EventKind::FileDeleted => "FILE_DELETE",
            EventKind::SnapshotCaptured => "SNAP_CAPTURE",
            EventKind::SnapshotCompared => "SNAP_DIFF",
            EventKind::CommFailure => "COMM_FAIL",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "PROC_START" => Some(EventKind::ProcessStarted),
            "PROC_PAUSE" => Some(EventKind::ProcessPaused),
            "PROC_RESUME" => Some(EventKind::ProcessResumed),
            "PROC_EXIT" => Some(EventKind::ProcessExited),
            "DBG_ATTACH" => Some(EventKind::DebuggerAttached),
            "DBG_DETACH" => Some(EventKind::DebuggerDisconnected),
            "FILE_CREATE" => Some(EventKind::FileCreated),
            "FILE_MODIFY" => Some(EventKind::FileModified),
            "FILE_DELETE" => Some(EventKind::FileDeleted),
            "SNAP_CAPTURE" => Some(EventKind::SnapshotCaptured),
            "SNAP_DIFF" => Some(EventKind::SnapshotCompared),
            "COMM_FAIL" => Some(EventKind::CommFailure),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DebugEvent {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub session_id: String,
    pub pid: u32,
    pub kind: EventKind,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub enum ClientMessage {
    Hello {
        version: u32,
        client_pid: u32,
    },
    Resume {
        pid: u32,
        mode: ResumeMode,
        target_depth: usize,
        break_on_err: bool,
        bps: Vec<(PathBuf, usize)>,
        cond_expr: String,
        force_tty: bool,
    },
    Skip {
        pid: u32,
    },
    Eval {
        pid: u32,
        expr: String,
    },
    SendStdin {
        input: String,
    },
    TtyGranted {
        pid: u32,
    },
    TakeSnapshot {
        scope: PathBuf,
    },
    SelectProcess {
        pid: u32,
    },
    Disconnect,
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum ServerMessage {
    Welcome {
        version: u32,
        session_id: String,
        root_pid: u32,
        entry_script: PathBuf,
    },
    Frame(RawFrame),
    ProcessList(Vec<ProcessMeta>),
    Event(DebugEvent),
    Stdout(String),
    Stderr(String),
    TtyHandoff {
        pid: u32,
        cmd: String,
    },
    TtyDone {
        pid: u32,
    },
    SessionExited {
        exit_code: i32,
    },
    Error {
        message: String,
    },
}

pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let _ = write!(out, "{:02x}", b);
    }
    out
}

pub fn hex_decode(hex: &str) -> Option<String> {
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let raw = hex.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        let hi = (raw[i] as char).to_digit(16)?;
        let lo = (raw[i + 1] as char).to_digit(16)?;
        bytes.push(((hi << 4) | lo) as u8);
        i += 2;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

impl ClientMessage {
    pub fn encode(&self) -> String {
        match self {
            ClientMessage::Hello {
                version,
                client_pid,
            } => format!("PEEK_V1\tHELLO\t{}\t{}\n", version, client_pid),
            ClientMessage::Resume {
                pid,
                mode,
                target_depth,
                break_on_err,
                bps,
                cond_expr,
                force_tty,
            } => {
                let bp_str = bps
                    .iter()
                    .map(|(p, l)| format!("{}:{}", hex_encode(p.to_string_lossy().as_bytes()), l))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "PEEK_V1\tRESUME\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    pid,
                    mode.as_protocol_str(),
                    target_depth,
                    if *break_on_err { 1 } else { 0 },
                    if *force_tty { 1 } else { 0 },
                    bp_str,
                    hex_encode(cond_expr.as_bytes())
                )
            }
            ClientMessage::Skip { pid } => format!("PEEK_V1\tSKIP\t{}\n", pid),
            ClientMessage::Eval { pid, expr } => {
                format!(
                    "PEEK_V1\tEVAL\t{}\t{}\n",
                    pid,
                    hex_encode(expr.as_bytes())
                )
            }
            ClientMessage::SendStdin { input } => {
                format!("PEEK_V1\tSTDIN\t{}\n", hex_encode(input.as_bytes()))
            }
            ClientMessage::TtyGranted { pid } => format!("PEEK_V1\tTTY_GRANTED\t{}\n", pid),
            ClientMessage::TakeSnapshot { scope } => {
                format!(
                    "PEEK_V1\tSNAPSHOT\t{}\n",
                    hex_encode(scope.to_string_lossy().as_bytes())
                )
            }
            ClientMessage::SelectProcess { pid } => format!("PEEK_V1\tSELECT_PID\t{}\n", pid),
            ClientMessage::Disconnect => "PEEK_V1\tDISCONNECT\n".to_string(),
            ClientMessage::Shutdown => "PEEK_V1\tSHUTDOWN\n".to_string(),
        }
    }

    pub fn decode(line: &str) -> Result<Self, String> {
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
        let parts: Vec<&str> = trimmed.split('\t').collect();
        if parts.len() < 2 || parts[0] != "PEEK_V1" {
            return Err("invalid protocol header".into());
        }
        match parts[1] {
            "HELLO" if parts.len() >= 4 => {
                let version = parts[2].parse().map_err(|_| "invalid version")?;
                let client_pid = parts[3].parse().map_err(|_| "invalid client_pid")?;
                Ok(ClientMessage::Hello {
                    version,
                    client_pid,
                })
            }
            "RESUME" if parts.len() >= 9 => {
                let pid = parts[2].parse().map_err(|_| "invalid pid")?;
                let mode = ResumeMode::from_protocol_str(parts[3])
                    .ok_or_else(|| "invalid resume mode".to_string())?;
                let target_depth = parts[4].parse().map_err(|_| "invalid depth")?;
                let break_on_err = parts[5] == "1";
                let force_tty = parts[6] == "1";
                let mut bps = Vec::new();
                if !parts[7].is_empty() {
                    for item in parts[7].split(',') {
                        if let Some((hpath, lstr)) = item.split_once(':') {
                            if let (Some(pstr), Ok(lno)) = (hex_decode(hpath), lstr.parse()) {
                                bps.push((PathBuf::from(pstr), lno));
                            }
                        }
                    }
                }
                let cond_expr = hex_decode(parts[8]).ok_or_else(|| "invalid cond hex".to_string())?;
                Ok(ClientMessage::Resume {
                    pid,
                    mode,
                    target_depth,
                    break_on_err,
                    bps,
                    cond_expr,
                    force_tty,
                })
            }
            "SKIP" if parts.len() >= 3 => {
                let pid = parts[2].parse().map_err(|_| "invalid pid")?;
                Ok(ClientMessage::Skip { pid })
            }
            "EVAL" if parts.len() >= 4 => {
                let pid = parts[2].parse().map_err(|_| "invalid pid")?;
                let expr = hex_decode(parts[3]).ok_or_else(|| "invalid eval hex".to_string())?;
                Ok(ClientMessage::Eval { pid, expr })
            }
            "STDIN" if parts.len() >= 3 => {
                let input = hex_decode(parts[2]).ok_or_else(|| "invalid stdin hex".to_string())?;
                Ok(ClientMessage::SendStdin { input })
            }
            "TTY_GRANTED" if parts.len() >= 3 => {
                let pid = parts[2].parse().map_err(|_| "invalid pid")?;
                Ok(ClientMessage::TtyGranted { pid })
            }
            "SNAPSHOT" if parts.len() >= 3 => {
                let s = hex_decode(parts[2]).ok_or_else(|| "invalid path hex".to_string())?;
                Ok(ClientMessage::TakeSnapshot {
                    scope: PathBuf::from(s),
                })
            }
            "SELECT_PID" if parts.len() >= 3 => {
                let pid = parts[2].parse().map_err(|_| "invalid pid")?;
                Ok(ClientMessage::SelectProcess { pid })
            }
            "DISCONNECT" => Ok(ClientMessage::Disconnect),
            "SHUTDOWN" => Ok(ClientMessage::Shutdown),
            other => Err(format!("unknown client command: {}", other)),
        }
    }
}

impl ServerMessage {
    pub fn encode(&self) -> String {
        match self {
            ServerMessage::Welcome {
                version,
                session_id,
                root_pid,
                entry_script,
            } => format!(
                "PEEK_V1\tWELCOME\t{}\t{}\t{}\t{}\n",
                version,
                hex_encode(session_id.as_bytes()),
                root_pid,
                hex_encode(entry_script.to_string_lossy().as_bytes())
            ),
            ServerMessage::Frame(f) => {
                let pipe_str = f
                    .pipe_status
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let stack_str = f
                    .stack
                    .iter()
                    .map(|sf| {
                        format!(
                            "{}:{}:{}",
                            hex_encode(sf.func.as_bytes()),
                            hex_encode(sf.file.to_string_lossy().as_bytes()),
                            sf.line
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let vars_str = f
                    .vars
                    .iter()
                    .map(|(k, (kind, val))| {
                        format!(
                            "{}:{}:{}",
                            hex_encode(k.as_bytes()),
                            kind.tag().trim(),
                            hex_encode(val.as_bytes())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "PEEK_V1\tFRAME\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    if f.is_sync { 1 } else { 0 },
                    f.pid,
                    f.ppid,
                    hex_encode(f.file.to_string_lossy().as_bytes()),
                    f.line,
                    f.exit_code,
                    f.subshell,
                    pipe_str,
                    hex_encode(f.pwd.as_bytes()),
                    hex_encode(f.cmd.as_bytes()),
                    stack_str,
                    vars_str
                )
            }
            ServerMessage::ProcessList(list) => {
                let items = list
                    .iter()
                    .map(|p| {
                        let st = match p.status {
                            ProcessStatus::Paused => "P".to_string(),
                            ProcessStatus::Running => "R".to_string(),
                            ProcessStatus::Exited(c) => format!("E:{}", c),
                            ProcessStatus::Disconnected => "D".to_string(),
                        };
                        format!(
                            "{}:{}:{}:{}:{}",
                            p.pid,
                            p.ppid,
                            hex_encode(p.role.as_bytes()),
                            hex_encode(p.script.to_string_lossy().as_bytes()),
                            st
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!("PEEK_V1\tPROCS\t{}\n", items)
            }
            ServerMessage::Event(ev) => format!(
                "PEEK_V1\tEVENT\t{}\t{}\t{}\t{}\t{}\t{}\n",
                ev.seq,
                ev.timestamp_ms,
                hex_encode(ev.session_id.as_bytes()),
                ev.pid,
                ev.kind.as_str(),
                hex_encode(ev.detail.as_bytes())
            ),
            ServerMessage::Stdout(line) => {
                format!("PEEK_V1\tSTDOUT\t{}\n", hex_encode(line.as_bytes()))
            }
            ServerMessage::Stderr(line) => {
                format!("PEEK_V1\tSTDERR\t{}\n", hex_encode(line.as_bytes()))
            }
            ServerMessage::TtyHandoff { pid, cmd } => {
                format!(
                    "PEEK_V1\tTTY_HANDOFF\t{}\t{}\n",
                    pid,
                    hex_encode(cmd.as_bytes())
                )
            }
            ServerMessage::TtyDone { pid } => format!("PEEK_V1\tTTY_DONE\t{}\n", pid),
            ServerMessage::SessionExited { exit_code } => {
                format!("PEEK_V1\tEXITED\t{}\n", exit_code)
            }
            ServerMessage::Error { message } => {
                format!("PEEK_V1\tERROR\t{}\n", hex_encode(message.as_bytes()))
            }
        }
    }

    pub fn decode(line: &str) -> Result<Self, String> {
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
        let parts: Vec<&str> = trimmed.split('\t').collect();
        if parts.len() < 2 || parts[0] != "PEEK_V1" {
            return Err("invalid server message header".into());
        }
        match parts[1] {
            "WELCOME" if parts.len() >= 6 => {
                let version = parts[2].parse().map_err(|_| "invalid version")?;
                let session_id = hex_decode(parts[3]).ok_or("invalid session_id")?;
                let root_pid = parts[4].parse().map_err(|_| "invalid root_pid")?;
                let entry_script =
                    PathBuf::from(hex_decode(parts[5]).ok_or("invalid entry_script")?);
                Ok(ServerMessage::Welcome {
                    version,
                    session_id,
                    root_pid,
                    entry_script,
                })
            }
            "FRAME" if parts.len() >= 14 => {
                let is_sync = parts[2] == "1";
                let pid = parts[3].parse().unwrap_or(0);
                let ppid = parts[4].parse().unwrap_or(0);
                let file = PathBuf::from(hex_decode(parts[5]).ok_or("invalid file hex")?);
                let line = parts[6].parse().unwrap_or(1);
                let exit_code = parts[7].parse().unwrap_or(0);
                let subshell = parts[8].parse().unwrap_or(0);
                let pipe_status = if parts[9].is_empty() {
                    Vec::new()
                } else {
                    parts[9]
                        .split(',')
                        .filter_map(|s| s.parse::<i32>().ok())
                        .collect()
                };
                let pwd = hex_decode(parts[10]).unwrap_or_default();
                let cmd = hex_decode(parts[11]).unwrap_or_default();
                let mut stack = Vec::new();
                if !parts[12].is_empty() {
                    for item in parts[12].split(',') {
                        let sp: Vec<&str> = item.splitn(3, ':').collect();
                        if sp.len() == 3 {
                            if let (Some(fn_name), Some(fpath), Ok(flin)) =
                                (hex_decode(sp[0]), hex_decode(sp[1]), sp[2].parse())
                            {
                                stack.push(StackFrame {
                                    func: fn_name,
                                    file: PathBuf::from(fpath),
                                    line: flin,
                                });
                            }
                        }
                    }
                }
                let mut vars = std::collections::BTreeMap::new();
                if !parts[13].is_empty() {
                    for item in parts[13].split(',') {
                        let vp: Vec<&str> = item.splitn(3, ':').collect();
                        if vp.len() == 3 {
                            if let (Some(vname), Some(vval)) =
                                (hex_decode(vp[0]), hex_decode(vp[2]))
                            {
                                let kind = match vp[1] {
                                    "env" => VarKind::Exported,
                                    "ro" => VarKind::ReadOnly,
                                    "int" => VarKind::Integer,
                                    "arr" => VarKind::Array,
                                    "map" => VarKind::AssocArray,
                                    _ => VarKind::Scalar,
                                };
                                vars.insert(vname, (kind, vval));
                            }
                        }
                    }
                }
                Ok(ServerMessage::Frame(RawFrame {
                    is_sync,
                    pid,
                    ppid,
                    file,
                    line,
                    exit_code,
                    subshell,
                    pipe_status,
                    pwd,
                    cmd,
                    stack,
                    vars,
                }))
            }
            "PROCS" if parts.len() >= 3 => {
                let mut list = Vec::new();
                if !parts[2].is_empty() {
                    for item in parts[2].split(',') {
                        let pp: Vec<&str> = item.splitn(5, ':').collect();
                        if pp.len() == 5 {
                            let pid = pp[0].parse().unwrap_or(0);
                            let ppid = pp[1].parse().unwrap_or(0);
                            let role = hex_decode(pp[2]).unwrap_or_default();
                            let script =
                                PathBuf::from(hex_decode(pp[3]).unwrap_or_default());
                            let status = if pp[4] == "P" {
                                ProcessStatus::Paused
                            } else if pp[4] == "R" {
                                ProcessStatus::Running
                            } else if let Some(c) = pp[4].strip_prefix("E:") {
                                ProcessStatus::Exited(c.parse().unwrap_or(0))
                            } else {
                                ProcessStatus::Disconnected
                            };
                            list.push(ProcessMeta {
                                pid,
                                ppid,
                                role,
                                script,
                                status,
                            });
                        }
                    }
                }
                Ok(ServerMessage::ProcessList(list))
            }
            "EVENT" if parts.len() >= 8 => {
                let seq = parts[2].parse().unwrap_or(0);
                let timestamp_ms = parts[3].parse().unwrap_or(0);
                let session_id = hex_decode(parts[4]).unwrap_or_default();
                let pid = parts[5].parse().unwrap_or(0);
                let kind = EventKind::from_str(parts[6]).ok_or("invalid event kind")?;
                let detail = hex_decode(parts[7]).unwrap_or_default();
                Ok(ServerMessage::Event(DebugEvent {
                    seq,
                    timestamp_ms,
                    session_id,
                    pid,
                    kind,
                    detail,
                }))
            }
            "STDOUT" if parts.len() >= 3 => {
                let s = hex_decode(parts[2]).ok_or("invalid stdout hex")?;
                Ok(ServerMessage::Stdout(s))
            }
            "STDERR" if parts.len() >= 3 => {
                let s = hex_decode(parts[2]).ok_or("invalid stderr hex")?;
                Ok(ServerMessage::Stderr(s))
            }
            "TTY_HANDOFF" if parts.len() >= 4 => {
                let pid = parts[2].parse().unwrap_or(0);
                let cmd = hex_decode(parts[3]).ok_or("invalid tty cmd hex")?;
                Ok(ServerMessage::TtyHandoff { pid, cmd })
            }
            "TTY_DONE" if parts.len() >= 3 => {
                let pid = parts[2].parse().unwrap_or(0);
                Ok(ServerMessage::TtyDone { pid })
            }
            "EXITED" if parts.len() >= 3 => {
                let exit_code = parts[2].parse().unwrap_or(0);
                Ok(ServerMessage::SessionExited { exit_code })
            }
            "ERROR" if parts.len() >= 3 => {
                let message = hex_decode(parts[2]).unwrap_or_default();
                Ok(ServerMessage::Error { message })
            }
            other => Err(format!("unknown server message: {}", other)),
        }
    }
}

pub fn read_bounded_message<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> io::Result<Option<String>> {
    buf.clear();
    let n = reader.read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > MAX_IPC_LINE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC frame exceeded maximum size limit",
        ));
    }
    Ok(Some(String::from_utf8_lossy(buf).into_owned()))
}

pub fn write_message<W: Write>(writer: &mut W, encoded: &str) -> io::Result<()> {
    writer.write_all(encoded.as_bytes())?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn roundtrips_client_messages() {
        let msg = ClientMessage::Resume {
            pid: 4242,
            mode: ResumeMode::Next,
            target_depth: 2,
            break_on_err: true,
            bps: vec![(PathBuf::from("/tmp/my script.sh"), 15)],
            cond_expr: "(( count > 5 ))".into(),
            force_tty: false,
        };
        let encoded = msg.encode();
        let decoded = ClientMessage::decode(&encoded).expect("decode failed");
        match decoded {
            ClientMessage::Resume {
                pid,
                mode,
                target_depth,
                break_on_err,
                bps,
                cond_expr,
                force_tty,
            } => {
                assert_eq!(pid, 4242);
                assert_eq!(mode, ResumeMode::Next);
                assert_eq!(target_depth, 2);
                assert!(break_on_err);
                assert!(!force_tty);
                assert_eq!(bps, vec![(PathBuf::from("/tmp/my script.sh"), 15)]);
                assert_eq!(cond_expr, "(( count > 5 ))");
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn roundtrips_server_frame_message() {
        let mut vars = BTreeMap::new();
        vars.insert(
            "GREETING".to_string(),
            (VarKind::Scalar, "hello\nworld\t!".to_string()),
        );
        let raw = RawFrame {
            is_sync: false,
            pid: 1001,
            ppid: 1000,
            file: PathBuf::from("/opt/deploy.sh"),
            line: 42,
            exit_code: 0,
            subshell: 1,
            pipe_status: vec![0, 1],
            pwd: "/opt".into(),
            cmd: "echo \"hello\"".into(),
            stack: vec![StackFrame {
                func: "run_task".into(),
                file: PathBuf::from("/opt/deploy.sh"),
                line: 10,
            }],
            vars,
        };
        let encoded = ServerMessage::Frame(raw).encode();
        let decoded = ServerMessage::decode(&encoded).expect("decode failed");
        match decoded {
            ServerMessage::Frame(f) => {
                assert_eq!(f.pid, 1001);
                assert_eq!(f.ppid, 1000);
                assert_eq!(f.line, 42);
                assert_eq!(f.pipe_status, vec![0, 1]);
                assert_eq!(f.stack.len(), 1);
                assert_eq!(f.stack[0].func, "run_task");
                assert_eq!(
                    f.vars.get("GREETING"),
                    Some(&(VarKind::Scalar, "hello\nworld\t!".to_string()))
                );
            }
            _ => panic!("unexpected server variant"),
        }
    }
}

// https://github.com/monji024

pub mod history;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::protocol::{DebugEvent, EventKind, ProcessMeta, ProcessStatus};
use self::history::HistoryBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Scalar,
    Exported,
    ReadOnly,
    Integer,
    Array,
    AssocArray,
}

impl VarKind {
    pub fn tag(self) -> &'static str {
        match self {
            VarKind::Scalar => "var",
            VarKind::Exported => "env",
            VarKind::ReadOnly => "ro ",
            VarKind::Integer => "int",
            VarKind::Array => "arr",
            VarKind::AssocArray => "map",
        }
    }
}

#[derive(Debug, Clone)]
pub struct VarEntry {
    pub name: String,
    pub value: String,
    pub prev_value: Option<String>,
    pub kind: VarKind,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct StackFrame {
    pub func: String,
    pub file: PathBuf,
    pub line: usize,
}

#[derive(Debug, Clone, Default)]
pub struct LineProfile {
    pub hits: usize,
    pub total_us: u128,
}

#[derive(Debug, Clone)]
pub enum LogStream {
    Stdout,
    Stderr,
    Eval,
    Stdin,
    Event,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub stream: LogStream,
    pub text: String,
}

pub struct BoundedLog {
    entries: VecDeque<LogEntry>,
    max_lines: usize,
}

impl BoundedLog {
    pub fn new(max_lines: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(1024),
            max_lines,
        }
    }

    pub fn push(&mut self, stream: LogStream, text: String) {
        if self.entries.len() >= self.max_lines {
            self.entries.pop_front();
        }
        self.entries.push_back(LogEntry { stream, text });
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &LogEntry> {
        self.entries.iter()
    }
}

pub struct EventBus {
    events: VecDeque<DebugEvent>,
    max_events: usize,
    next_seq: u64,
    session_id: String,
}

impl EventBus {
    pub fn new(session_id: String, max_events: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(512),
            max_events,
            next_seq: 1,
            session_id,
        }
    }

    pub fn record(&mut self, pid: u32, kind: EventKind, detail: impl Into<String>) -> DebugEvent {
        let detail_str = detail.into();
        if let Some(last) = self.events.back_mut() {
            if last.pid == pid && last.kind == kind && last.detail == detail_str {
                last.timestamp_ms = now_ms();
                return last.clone();
            }
        }
        let ev = DebugEvent {
            seq: self.next_seq,
            timestamp_ms: now_ms(),
            session_id: self.session_id.clone(),
            pid,
            kind,
            detail: detail_str,
        };
        self.next_seq += 1;
        if self.events.len() >= self.max_events {
            self.events.pop_front();
        }
        self.events.push_back(ev.clone());
        ev
    }

    pub fn push_external(&mut self, ev: DebugEvent) {
        if self.events.len() >= self.max_events {
            self.events.pop_front();
        }
        self.next_seq = self.next_seq.max(ev.seq + 1);
        self.events.push_back(ev);
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.next_seq = 1;
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &DebugEvent> {
        self.events.iter()
    }
}

pub struct ProcessTable {
    processes: BTreeMap<u32, ProcessMeta>,
    histories: BTreeMap<u32, HistoryBuffer>,
    active_pid: u32,
    history_cap: usize,
}

impl ProcessTable {
    pub fn new(root_pid: u32, entry_script: PathBuf, history_cap: usize) -> Self {
        let mut processes = BTreeMap::new();
        let mut histories = BTreeMap::new();
        processes.insert(
            root_pid,
            ProcessMeta {
                pid: root_pid,
                ppid: 0,
                role: "root".into(),
                script: entry_script,
                status: ProcessStatus::Running,
            },
        );
        histories.insert(root_pid, HistoryBuffer::new(history_cap));
        Self {
            processes,
            histories,
            active_pid: root_pid,
            history_cap,
        }
    }

    pub fn active_pid(&self) -> u32 {
        self.active_pid
    }

    pub fn set_active_pid(&mut self, pid: u32) -> bool {
        if self.processes.contains_key(&pid) {
            self.active_pid = pid;
            true
        } else {
            false
        }
    }

    pub fn cycle_next_process(&mut self) -> u32 {
        let pids: Vec<u32> = self.processes.keys().copied().collect();
        if pids.is_empty() {
            return self.active_pid;
        }
        let pos = pids.iter().position(|&p| p == self.active_pid).unwrap_or(0);
        let next = pids[(pos + 1) % pids.len()];
        self.active_pid = next;
        next
    }

    pub fn upsert_process(
        &mut self,
        pid: u32,
        ppid: u32,
        role: String,
        script: PathBuf,
        status: ProcessStatus,
    ) {
        self.processes
            .entry(pid)
            .and_modify(|m| {
                if ppid > 0 {
                    m.ppid = ppid;
                }
                if !role.is_empty() {
                    m.role = role.clone();
                }
                if !script.as_os_str().is_empty() {
                    m.script = script.clone();
                }
                m.status = status.clone();
            })
            .or_insert(ProcessMeta {
                pid,
                ppid,
                role,
                script,
                status,
            });
        self.histories
            .entry(pid)
            .or_insert_with(|| HistoryBuffer::new(self.history_cap));
    }

    pub fn set_status(&mut self, pid: u32, status: ProcessStatus) {
        if let Some(meta) = self.processes.get_mut(&pid) {
            meta.status = status;
        }
    }

    pub fn sync_from_list(&mut self, list: Vec<ProcessMeta>) {
        for p in list {
            self.upsert_process(p.pid, p.ppid, p.role, p.script, p.status);
        }
        if !self.processes.contains_key(&self.active_pid) {
            if let Some(&first) = self.processes.keys().next() {
                self.active_pid = first;
            }
        }
    }

    pub fn history_for_mut(&mut self, pid: u32) -> &mut HistoryBuffer {
        let cap = self.history_cap;
        self.histories
            .entry(pid)
            .or_insert_with(|| HistoryBuffer::new(cap))
    }

    pub fn active_history(&self) -> Option<&HistoryBuffer> {
        self.histories.get(&self.active_pid)
    }

    pub fn active_history_mut(&mut self) -> &mut HistoryBuffer {
        let pid = self.active_pid;
        self.history_for_mut(pid)
    }

    pub fn clear_histories(&mut self) {
        for hist in self.histories.values_mut() {
            hist.clear();
        }
    }

    pub fn list(&self) -> Vec<&ProcessMeta> {
        self.processes.values().collect()
    }

    pub fn len(&self) -> usize {
        self.processes.len()
    }
}

pub struct SourceCache {
    files: HashMap<PathBuf, Vec<String>>,
    profiles: HashMap<(PathBuf, usize), LineProfile>,
}

impl SourceCache {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            profiles: HashMap::new(),
        }
    }

    pub fn ensure_loaded(&mut self, path: &Path) -> &[String] {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.files.entry(canonical).or_insert_with(|| match std::fs::read(path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes)
                .lines()
                .map(String::from)
                .collect(),
            Err(e) => vec![format!("# [peek: unable to read {:?}: {}]", path, e)],
        })
    }

    pub fn get_lines(&self, path: &Path) -> &[String] {
        self.files.get(path).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn record_profile(&mut self, file: PathBuf, line: usize, elapsed: Duration) {
        let entry = self.profiles.entry((file, line)).or_default();
        entry.hits += 1;
        entry.total_us += elapsed.as_micros();
    }

    pub fn get_profile(&self, file: &Path, line: usize) -> Option<&LineProfile> {
        self.profiles.get(&(file.to_path_buf(), line))
    }

    pub fn clear_profiles(&mut self) {
        self.profiles.clear();
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
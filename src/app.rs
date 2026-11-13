// https://github.com/monji024

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};

use crate::engine::{parser::RawFrame, EngineEvent, ResumeMode, Session};
use crate::protocol::{ClientMessage, EventKind, ProcessStatus, ServerMessage};
use crate::security::{redact_command_args, redact_value};
use crate::session_mgr::RemoteClient;
use crate::snapshot::{FsSnapshot, SnapshotDiff};
use crate::state::{
    history::{HistoryBuffer, Snapshot},
    BoundedLog, EventBus, LogStream, ProcessTable, SourceCache, VarEntry, VarKind,
};
use crate::ui::theme::{ColorDepth, Palette, ThemeMode};

const MAX_LOG_LINES: usize = 5_000;
const MAX_HISTORY_FRAMES: usize = 256;
const MAX_EVENTS: usize = 1_024;
const MAX_FS_SNAPSHOTS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecState {
    Running,
    Paused,
    Exited(i32),
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneFocus {
    Source,
    Variables,
    Stack,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomTab {
    Console,
    Events,
    Snapshots,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    None,
    Eval,
    Filter,
    Watch,
    GotoLine,
    FindSource,
    SendStdin,
    CondBreak,
    SnapshotScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayModal {
    None,
    InspectVar,
    Help,
    Processes,
}

pub enum TtyAction {
    None,
    Acquire(String),
    Release,
}

pub enum SessionBackend {
    Local(Session),
    Remote(RemoteClient),
}

impl SessionBackend {
    pub fn pid(&self) -> u32 {
        match self {
            SessionBackend::Local(s) => s.pid(),
            SessionBackend::Remote(r) => r.root_pid,
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, SessionBackend::Remote(_))
    }

    pub fn shutdown(&mut self) {
        match self {
            SessionBackend::Local(s) => s.shutdown(),
            SessionBackend::Remote(r) => r.disconnect(),
        }
    }
}

pub struct App {
    pub session_id: String,
    pub entry_script: PathBuf,
    pub script_args: Vec<String>,
    pub viewed_file: PathBuf,
    pub sources: SourceCache,
    pub backend: SessionBackend,
    pub proc_table: ProcessTable,
    pub events: EventBus,
    pub fs_snapshots: Vec<FsSnapshot>,
    pub latest_diff: Option<SnapshotDiff>,
    pub snapshot_scope: Option<PathBuf>,
    pub logs: BoundedLog,
    pub breakpoints: HashSet<(PathBuf, usize)>,
    pub cond_breakpoint: String,
    pub break_on_err: bool,
    pub show_system_env: bool,
    pub redact_secrets: bool,
    pub theme_mode: ThemeMode,
    pub color_depth: ColorDepth,
    pub watches: Vec<String>,
    pub var_filter: String,
    pub source_query: String,
    pub exec_state: ExecState,
    pub focus: PaneFocus,
    pub bottom_tab: BottomTab,
    pub prompt_mode: PromptMode,
    pub modal: OverlayModal,
    pub prompt_buf: String,
    pub cursor_line: usize,
    pub source_scroll: usize,
    pub var_scroll: usize,
    pub stack_scroll: usize,
    pub output_scroll: usize,
    pub status: String,
    pub tty_passthrough: bool,
    step_counter: usize,
    step_timer: Instant,
}

impl App {
    pub fn new(
        entry_script: PathBuf,
        script_args: Vec<String>,
        initial_bp_lines: &[usize],
        break_on_err: bool,
        theme_mode: ThemeMode,
        snapshot_scope: Option<PathBuf>,
    ) -> Result<Self> {
        let mut sources = SourceCache::new();
        let line_count = sources.ensure_loaded(&entry_script).len().max(1);

        let mut breakpoints = HashSet::new();
        for &line in initial_bp_lines {
            if line >= 1 && line <= line_count {
                breakpoints.insert((entry_script.clone(), line));
            }
        }

        let bp_vec: Vec<_> = breakpoints.iter().cloned().collect();
        let session = Session::spawn(&entry_script, &script_args, &bp_vec, break_on_err)?;
        let root_pid = session.pid();
        let stem = entry_script
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("bash");
        let session_id = format!("{}-{}", stem, root_pid);

        let mut events = EventBus::new(session_id.clone(), MAX_EVENTS);
        events.record(
            root_pid,
            EventKind::DebuggerAttached,
            format!("Started local debugging session for {}", entry_script.display()),
        );

        Ok(Self {
            session_id,
            viewed_file: entry_script.clone(),
            entry_script: entry_script.clone(),
            script_args,
            sources,
            backend: SessionBackend::Local(session),
            proc_table: ProcessTable::new(root_pid, entry_script, MAX_HISTORY_FRAMES),
            events,
            fs_snapshots: Vec::new(),
            latest_diff: None,
            snapshot_scope,
            logs: BoundedLog::new(MAX_LOG_LINES),
            breakpoints,
            cond_breakpoint: String::new(),
            break_on_err,
            show_system_env: false,
            redact_secrets: true,
            theme_mode,
            color_depth: ColorDepth::detect(),
            watches: Vec::new(),
            var_filter: String::new(),
            source_query: String::new(),
            exec_state: ExecState::Running,
            focus: PaneFocus::Source,
            bottom_tab: BottomTab::Console,
            prompt_mode: PromptMode::None,
            modal: OverlayModal::None,
            prompt_buf: String::new(),
            cursor_line: 1,
            source_scroll: 0,
            var_scroll: 0,
            stack_scroll: 0,
            output_scroll: 0,
            status: "Attached to Bash process — press '?' for keyboard shortcuts".into(),
            tty_passthrough: false,
            step_counter: 0,
            step_timer: Instant::now(),
        })
    }

    pub fn new_attached(
        client: RemoteClient,
        initial_bp_lines: &[usize],
        break_on_err: bool,
        theme_mode: ThemeMode,
        snapshot_scope: Option<PathBuf>,
    ) -> Result<Self> {
        let entry_script = client.entry_script.clone();
        let root_pid = client.root_pid;
        let session_id = client.session_id.clone();

        let mut sources = SourceCache::new();
        let line_count = sources.ensure_loaded(&entry_script).len().max(1);

        let mut breakpoints = HashSet::new();
        for &line in initial_bp_lines {
            if line >= 1 && line <= line_count {
                breakpoints.insert((entry_script.clone(), line));
            }
        }

        let mut events = EventBus::new(session_id.clone(), MAX_EVENTS);
        events.record(
            root_pid,
            EventKind::DebuggerAttached,
            format!("Attached to remote session '{}'", session_id),
        );

        Ok(Self {
            session_id: session_id.clone(),
            viewed_file: entry_script.clone(),
            entry_script: entry_script.clone(),
            script_args: Vec::new(),
            sources,
            backend: SessionBackend::Remote(client),
            proc_table: ProcessTable::new(root_pid, entry_script, MAX_HISTORY_FRAMES),
            events,
            fs_snapshots: Vec::new(),
            latest_diff: None,
            snapshot_scope,
            logs: BoundedLog::new(MAX_LOG_LINES),
            breakpoints,
            cond_breakpoint: String::new(),
            break_on_err,
            show_system_env: false,
            redact_secrets: true,
            theme_mode,
            color_depth: ColorDepth::detect(),
            watches: Vec::new(),
            var_filter: String::new(),
            source_query: String::new(),
            exec_state: ExecState::Running,
            focus: PaneFocus::Source,
            bottom_tab: BottomTab::Console,
            prompt_mode: PromptMode::None,
            modal: OverlayModal::None,
            prompt_buf: String::new(),
            cursor_line: 1,
            source_scroll: 0,
            var_scroll: 0,
            stack_scroll: 0,
            output_scroll: 0,
            status: format!(
                "Connected to session '{}' (PID {}) — press '?' for shortcuts",
                session_id, root_pid
            ),
            tty_passthrough: false,
            step_counter: 0,
            step_timer: Instant::now(),
        })
    }

    pub fn palette(&self) -> Palette {
        Palette::for_mode(self.theme_mode, self.color_depth)
    }

    pub fn pid(&self) -> u32 {
        self.proc_table.active_pid()
    }

    pub fn active_history(&self) -> Option<&HistoryBuffer> {
        self.proc_table.active_history()
    }

    pub fn send_tty_granted(&mut self) -> Result<()> {
        let pid = self.pid();
        match &mut self.backend {
            SessionBackend::Local(s) => s.send_tty_granted(),
            SessionBackend::Remote(r) => r.send(&ClientMessage::TtyGranted { pid }),
        }
    }

    pub fn sync_viewport(&mut self, visible_rows: usize) {
        if visible_rows == 0 {
            return;
        }
        let margin = 3.min(visible_rows / 4);
        let idx = self.cursor_line.saturating_sub(1);

        if idx < self.source_scroll + margin {
            self.source_scroll = idx.saturating_sub(margin);
        } else if idx >= self.source_scroll + visible_rows.saturating_sub(margin) {
            self.source_scroll = idx.saturating_sub(visible_rows.saturating_sub(margin + 1));
        }
    }

    pub fn tick(&mut self) -> Result<TtyAction> {
        let mut action = TtyAction::None;

        let mut local_events = Vec::new();
        let mut remote_msgs = Vec::new();

        match &self.backend {
            SessionBackend::Local(s) => {
                while let Some(ev) = s.try_recv() {
                    local_events.push(ev);
                }
            }
            SessionBackend::Remote(r) => {
                while let Some(msg) = r.try_recv() {
                    remote_msgs.push(msg);
                }
            }
        }

        for ev in local_events {
            match ev {
                EngineEvent::Frame(raw) => {
                    if self.tty_passthrough {
                        self.tty_passthrough = false;
                        action = TtyAction::Release;
                    }
                    self.ingest_frame(raw);
                }
                EngineEvent::ProcessStarted {
                    pid,
                    ppid,
                    role,
                    script,
                } => {
                    self.proc_table.upsert_process(
                        pid,
                        ppid,
                        role.clone(),
                        script.clone(),
                        ProcessStatus::Running,
                    );
                    let dev = self.events.record(
                        pid,
                        EventKind::ProcessStarted,
                        format!("{} ({}) ppid={}", script.display(), role, ppid),
                    );
                    self.logs.push(
                        LogStream::Event,
                        format!("[event] PID {} {}: {}", dev.pid, dev.kind.as_str(), dev.detail),
                    );
                }
                EngineEvent::ProcessExited { pid, exit_code } => {
                    self.proc_table
                        .set_status(pid, ProcessStatus::Exited(exit_code));
                    let dev = self.events.record(
                        pid,
                        EventKind::ProcessExited,
                        format!("exited with status {}", exit_code),
                    );
                    self.logs.push(
                        LogStream::Event,
                        format!("[event] PID {} {}: {}", dev.pid, dev.kind.as_str(), dev.detail),
                    );
                }
                EngineEvent::TtyHandoff(cmd) => {
                    self.tty_passthrough = true;
                    return Ok(TtyAction::Acquire(cmd));
                }
                EngineEvent::TtyDone => {
                    if self.tty_passthrough {
                        self.tty_passthrough = false;
                        action = TtyAction::Release;
                    }
                }
                EngineEvent::Stdout(line) => {
                    self.logs.push(LogStream::Stdout, line);
                    if self.bottom_tab == BottomTab::Console {
                        self.output_scroll = self.logs.len().saturating_sub(1);
                    }
                }
                EngineEvent::Stderr(line) => {
                    self.logs
                        .push(LogStream::Stderr, format!("[stderr] {}", line));
                    if self.bottom_tab == BottomTab::Console {
                        self.output_scroll = self.logs.len().saturating_sub(1);
                    }
                }
            }
        }

        for msg in remote_msgs {
            match msg {
                ServerMessage::Welcome { .. } => {}
                ServerMessage::Frame(raw) => {
                    self.ingest_frame(raw);
                }
                ServerMessage::ProcessList(list) => {
                    self.proc_table.sync_from_list(list);
                }
                ServerMessage::Event(ev) => {
                    self.logs.push(
                        LogStream::Event,
                        format!("[event] PID {} {}: {}", ev.pid, ev.kind.as_str(), ev.detail),
                    );
                    self.events.push_external(ev);
                }
                ServerMessage::Stdout(line) => {
                    self.logs.push(LogStream::Stdout, line);
                    if self.bottom_tab == BottomTab::Console {
                        self.output_scroll = self.logs.len().saturating_sub(1);
                    }
                }
                ServerMessage::Stderr(line) => {
                    self.logs
                        .push(LogStream::Stderr, format!("[stderr] {}", line));
                    if self.bottom_tab == BottomTab::Console {
                        self.output_scroll = self.logs.len().saturating_sub(1);
                    }
                }
                ServerMessage::TtyHandoff { cmd, .. } => {
                    self.status = format!("Remote process requested TTY for: {}", cmd);
                }
                ServerMessage::TtyDone { .. } => {}
                ServerMessage::SessionExited { exit_code } => {
                    self.exec_state = ExecState::Exited(exit_code);
                    self.status = format!(
                        "Remote script finished with exit code {} ('[' time-travel, 'q' detach)",
                        exit_code
                    );
                }
                ServerMessage::Error { message } => {
                    self.exec_state = ExecState::Disconnected;
                    self.events.record(
                        self.pid(),
                        EventKind::DebuggerDisconnected,
                        message.clone(),
                    );
                    self.status = format!("Disconnected: {}", message);
                }
            }
        }

        if let SessionBackend::Local(ref mut session) = self.backend {
            if !matches!(self.exec_state, ExecState::Exited(_)) {
                if let Some(code) = session.try_wait_exit()? {
                    if self.tty_passthrough {
                        self.tty_passthrough = false;
                        action = TtyAction::Release;
                    }
                    let root_pid = session.pid();
                    self.proc_table
                        .set_status(root_pid, ProcessStatus::Exited(code));
                    self.exec_state = ExecState::Exited(code);
                    self.status = format!(
                        "Script finished with exit code {} ('r' restart, '[' time-travel, 'q' quit)",
                        code
                    );
                }
            }
        }

        Ok(action)
    }

    fn ingest_frame(&mut self, raw: RawFrame) {
        let canonical = std::fs::canonicalize(&raw.file).unwrap_or(raw.file);
        self.sources.ensure_loaded(&canonical);

        let fpid = if raw.pid > 0 {
            raw.pid
        } else {
            self.backend.pid()
        };
        self.proc_table.upsert_process(
            fpid,
            raw.ppid,
            if fpid == self.backend.pid() {
                "root".into()
            } else {
                "child".into()
            },
            canonical.clone(),
            ProcessStatus::Paused,
        );
        self.proc_table.set_active_pid(fpid);

        let latency = self.step_timer.elapsed();
        if !raw.is_sync {
            if let Some(prev) = self.proc_table.active_history().and_then(|h| h.active()) {
                self.sources
                    .record_profile(prev.file.clone(), prev.line, latency);
            }
        }

        let prev_vars = self
            .proc_table
            .active_history()
            .and_then(|h| h.active())
            .map(|s| s.vars.clone());
        let mut vars = BTreeMap::new();

        for (name, (kind, value)) in raw.vars {
            let prev_val = prev_vars
                .as_ref()
                .and_then(|m| m.get(&name))
                .map(|old| old.value.clone());
            let changed = match &prev_val {
                Some(old) => old != &value,
                None => self.step_counter > 0 || raw.is_sync,
            };
            vars.insert(
                name.clone(),
                VarEntry {
                    name,
                    value,
                    prev_value: prev_val,
                    kind,
                    changed,
                },
            );
        }

        if !raw.is_sync {
            self.step_counter += 1;
            self.events.record(
                fpid,
                EventKind::ProcessPaused,
                format!(
                    "{}:{} ({})",
                    canonical
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("script"),
                    raw.line,
                    redact_command_args(&raw.cmd)
                ),
            );
        }

        let max_line = self.sources.get_lines(&canonical).len().max(1);
        let clamped_line = raw.line.clamp(1, max_line);

        let snapshot = Snapshot {
            step_index: self.step_counter,
            file: canonical.clone(),
            line: clamped_line,
            exit_code: raw.exit_code,
            subshell: raw.subshell,
            pipe_status: raw.pipe_status,
            pwd: raw.pwd,
            cmd: raw.cmd.clone(),
            stack: raw.stack,
            vars,
            latency,
        };

        self.viewed_file = canonical;
        self.cursor_line = clamped_line;
        self.proc_table.active_history_mut().push(snapshot);
        self.exec_state = ExecState::Paused;

        if !raw.is_sync {
            let sub_badge = if raw.subshell > 0 {
                format!(" [subshell #{}]", raw.subshell)
            } else {
                String::new()
            };
            let pid_badge = if self.proc_table.len() > 1 {
                format!(" [PID {}]", fpid)
            } else {
                String::new()
            };
            self.status = format!(
                "line {}{}{} → {}",
                clamped_line,
                pid_badge,
                sub_badge,
                redact_command_args(&raw.cmd)
            );
        }
    }

    pub fn capture_fs_snapshot(&mut self, custom_scope: Option<PathBuf>) -> Result<()> {
        let scope = custom_scope
            .or_else(|| self.snapshot_scope.clone())
            .or_else(|| {
                self.active_history()
                    .and_then(|h| h.active())
                    .map(|s| PathBuf::from(&s.pwd))
            })
            .unwrap_or_else(|| {
                self.entry_script
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            });

        self.snapshot_scope = Some(scope.clone());
        let id = self.fs_snapshots.len() + 1;
        let pid = self.pid();
        let step = self.step_counter;

        match FsSnapshot::capture(id, pid, step, &scope) {
            Ok(snap) => {
                let file_count = snap.entries.len();
                let ev = self.events.record(
                    pid,
                    EventKind::SnapshotCaptured,
                    format!(
                        "snapshot #{} ({} files in {})",
                        id,
                        file_count,
                        snap.root_scope.display()
                    ),
                );
                self.logs.push(
                    LogStream::Event,
                    format!("[snapshot] {}", ev.detail),
                );

                if let Some(prev) = self.fs_snapshots.last() {
                    let diff = FsSnapshot::compare(prev, &snap);
                    for ch in &diff.changes {
                        let ek = match ch.kind {
                            crate::snapshot::DiffKind::Added => EventKind::FileCreated,
                            crate::snapshot::DiffKind::Modified => EventKind::FileModified,
                            crate::snapshot::DiffKind::Deleted => EventKind::FileDeleted,
                        };
                        self.events.record(
                            pid,
                            ek,
                            format!("{} {}", ch.kind.symbol(), ch.rel_path.display()),
                        );
                    }
                    self.events.record(
                        pid,
                        EventKind::SnapshotCompared,
                        format!(
                            "snapshot #{} vs #{}: {} changes",
                            prev.id,
                            snap.id,
                            diff.changes.len()
                        ),
                    );
                    self.status = format!(
                        "Captured snapshot #{} ({} files, {} changes since #{})",
                        id,
                        file_count,
                        diff.changes.len(),
                        prev.id
                    );
                    self.latest_diff = Some(diff);
                } else {
                    self.status = format!(
                        "Captured baseline snapshot #{} ({} files in {})",
                        id,
                        file_count,
                        snap.root_scope.display()
                    );
                }

                if self.fs_snapshots.len() >= MAX_FS_SNAPSHOTS {
                    self.fs_snapshots.remove(0);
                }
                self.fs_snapshots.push(snap);
                self.bottom_tab = BottomTab::Snapshots;
            }
            Err(e) => {
                self.status = format!("Snapshot failed: {}", e);
            }
        }
        Ok(())
    }

    pub fn resume(&mut self, mode: ResumeMode, force_tty: bool) -> Result<()> {
        if self.exec_state != ExecState::Paused {
            return Ok(());
        }
        self.proc_table.active_history_mut().return_to_live();
        let depth = self
            .active_history()
            .and_then(|h| h.active())
            .map(|s| s.stack.len())
            .unwrap_or(0);
        let bps: Vec<_> = self.breakpoints.iter().cloned().collect();
        let pid = self.pid();

        self.step_timer = Instant::now();
        self.exec_state = ExecState::Running;
        self.proc_table.set_status(pid, ProcessStatus::Running);
        self.events
            .record(pid, EventKind::ProcessResumed, mode.as_protocol_str());

        match &mut self.backend {
            SessionBackend::Local(s) => s.send_resume(
                mode,
                depth,
                self.break_on_err,
                &bps,
                &self.cond_breakpoint,
                force_tty,
            )?,
            SessionBackend::Remote(r) => r.send(&ClientMessage::Resume {
                pid,
                mode,
                target_depth: depth,
                break_on_err: self.break_on_err,
                bps,
                cond_expr: self.cond_breakpoint.clone(),
                force_tty,
            })?,
        }
        Ok(())
    }

    pub fn skip_current_command(&mut self) -> Result<()> {
        if self.exec_state != ExecState::Paused {
            return Ok(());
        }
        self.proc_table.active_history_mut().return_to_live();
        let pid = self.pid();
        self.step_timer = Instant::now();
        self.exec_state = ExecState::Running;
        self.proc_table.set_status(pid, ProcessStatus::Running);
        match &mut self.backend {
            SessionBackend::Local(s) => s.send_skip()?,
            SessionBackend::Remote(r) => r.send(&ClientMessage::Skip { pid })?,
        }
        self.status = "Skipped execution of line (extdebug dry-run)".into();
        Ok(())
    }

    pub fn restart(&mut self) -> Result<()> {
        if self.backend.is_remote() {
            self.status = "Restart is only available for local sessions".into();
            return Ok(());
        }
        self.backend.shutdown();
        self.proc_table.clear_histories();
        self.logs.clear();
        self.events.clear();
        self.sources.clear_profiles();
        self.step_counter = 0;
        self.tty_passthrough = false;
        self.step_timer = Instant::now();

        let bps: Vec<_> = self.breakpoints.iter().cloned().collect();
        let session = Session::spawn(
            &self.entry_script,
            &self.script_args,
            &bps,
            self.break_on_err,
        )?;
        let root_pid = session.pid();
        self.proc_table = ProcessTable::new(root_pid, self.entry_script.clone(), MAX_HISTORY_FRAMES);
        self.backend = SessionBackend::Local(session);
        self.exec_state = ExecState::Running;
        self.status = "Session restarted cleanly".into();
        Ok(())
    }

    pub fn export_markdown_report(&mut self) -> Result<()> {
        let Some(snap) = self.active_history().and_then(|h| h.active()).cloned() else {
            self.status = "No snapshot available to export".into();
            return Ok(());
        };

        let mut report = String::new();
        let _ = writeln!(report, "# peek Diagnostic Snapshot");
        let _ = writeln!(report, "- **Session:** `{}` (PID `{}`)", self.session_id, self.pid());
        let _ = writeln!(report, "- **Script:** `{}`", snap.file.display());
        let _ = writeln!(
            report,
            "- **Line:** `{}` (`{}`)",
            snap.line,
            redact_command_args(&snap.cmd)
        );
        let _ = writeln!(report, "- **Last Status (`$?`):** `{}`", snap.exit_code);
        let _ = writeln!(report, "- **Subshell Level:** `{}`", snap.subshell);
        let _ = writeln!(report, "- **Working Directory:** `{}`\n", snap.pwd);

        let _ = writeln!(report, "## Call Stack");
        if snap.stack.is_empty() {
            let _ = writeln!(report, "- `<script-root>` at line {}", snap.line);
        } else {
            for (i, f) in snap.stack.iter().enumerate() {
                let _ = writeln!(
                    report,
                    "- `#{}` `{}` at `{}:{}`",
                    i,
                    f.func,
                    f.file.display(),
                    f.line
                );
            }
        }

        let _ = writeln!(report, "\n## Mutated & Script Variables");
        for v in snap
            .vars
            .values()
            .filter(|v| v.kind != VarKind::Exported || v.changed)
        {
            let delta = if v.changed { " (Δ mutated)" } else { "" };
            let display_val = if self.redact_secrets {
                redact_value(&v.name, &v.value)
            } else {
                v.value.clone()
            };
            let _ = writeln!(
                report,
                "- `{}` [{}]{} = `{:?}`",
                v.name,
                v.kind.tag(),
                delta,
                display_val
            );
        }

        if let Some(ref diff) = self.latest_diff {
            let _ = writeln!(
                report,
                "\n## File Snapshot Diff (#{} → #{})",
                diff.from_id, diff.to_id
            );
            for ch in &diff.changes {
                let _ = writeln!(report, "- `{}` `{}`", ch.kind.symbol(), ch.rel_path.display());
            }
        }

        std::fs::write("peek-report.md", report)?;
        self.status = "Exported diagnostic snapshot to ./peek-report.md".into();
        Ok(())
    }

    pub fn display_var_value(&self, name: &str, raw_value: &str) -> String {
        if self.redact_secrets {
            redact_value(name, raw_value)
        } else {
            raw_value.to_string()
        }
    }

    pub fn active_source_lines(&self) -> &[String] {
        self.sources.get_lines(&self.viewed_file)
    }

    pub fn has_breakpoint(&self, file: &Path, line: usize) -> bool {
        self.breakpoints.contains(&(file.to_path_buf(), line))
    }

    pub fn toggle_breakpoint_at(&mut self, line: usize) {
        let max = self.active_source_lines().len().max(1);
        let target = line.clamp(1, max);
        let key = (self.viewed_file.clone(), target);
        if self.breakpoints.remove(&key) {
            self.status = format!("Removed breakpoint at line {}", target);
        } else {
            self.breakpoints.insert(key);
            self.status = format!("Added breakpoint ● at line {}", target);
        }
    }

    pub fn find_next_in_source(&mut self, from_line: usize) {
        if self.source_query.is_empty() {
            return;
        }
        let needle = self.source_query.to_lowercase();
        let lines = self.active_source_lines();
        if lines.is_empty() {
            return;
        }

        for offset in 0..lines.len() {
            let idx = (from_line + offset) % lines.len();
            if lines[idx].to_lowercase().contains(&needle) {
                self.cursor_line = idx + 1;
                self.status = format!(
                    "Match for '{}' at line {}",
                    self.source_query, self.cursor_line
                );
                return;
            }
        }
        self.status = format!("Pattern '{}' not found in source", self.source_query);
    }

    pub fn filtered_vars(&self) -> Vec<&VarEntry> {
        let Some(snap) = self.active_history().and_then(|h| h.active()) else {
            return Vec::new();
        };
        let pat = self.var_filter.to_lowercase();
        snap.vars
            .values()
            .filter(|v| {
                if !self.show_system_env && v.kind == VarKind::Exported && !v.changed {
                    return false;
                }
                pat.is_empty()
                    || v.name.to_lowercase().contains(&pat)
                    || v.value.to_lowercase().contains(&pat)
            })
            .collect()
    }

    pub fn selected_var(&self) -> Option<&VarEntry> {
        let vars = self.filtered_vars();
        vars.get(self.var_scroll).copied()
    }

    pub fn handle_mouse(&mut self, ev: MouseEvent) {
        match ev.kind {
            MouseEventKind::ScrollUp => self.scroll_up(3),
            MouseEventKind::ScrollDown => self.scroll_down(3),
            MouseEventKind::Down(MouseButton::Left) => {
                if ev.row >= 2 && ev.column < 16 {
                    let clicked_line =
                        self.source_scroll + (ev.row as usize).saturating_sub(2) + 1;
                    if clicked_line <= self.active_source_lines().len() {
                        self.cursor_line = clicked_line;
                        self.toggle_breakpoint_at(clicked_line);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn handle_key(&mut self, code: KeyCode) -> Result<bool> {
        if self.modal != OverlayModal::None {
            if matches!(
                code,
                KeyCode::Esc
                    | KeyCode::Enter
                    | KeyCode::Char('q')
                    | KeyCode::Char('?')
                    | KeyCode::Char('P')
            ) {
                self.modal = OverlayModal::None;
            }
            return Ok(false);
        }

        if self.prompt_mode != PromptMode::None {
            self.handle_prompt_key(code)?;
            return Ok(false);
        }

        match code {
            KeyCode::Char('q') => {
                self.backend.shutdown();
                return Ok(true);
            }
            KeyCode::Char('?') => self.modal = OverlayModal::Help,
            KeyCode::Char('P') => {
                let next_pid = self.proc_table.cycle_next_process();
                if let Some(snap) = self.active_history().and_then(|h| h.active()).cloned() {
                    self.viewed_file = snap.file;
                    self.cursor_line = snap.line;
                }
                self.modal = OverlayModal::Processes;
                self.status = format!(
                    "Active process PID {} ({} tracked processes)",
                    next_pid,
                    self.proc_table.len()
                );
            }
            KeyCode::Enter => {
                if !self.filtered_vars().is_empty() {
                    self.modal = OverlayModal::InspectVar;
                }
            }
            KeyCode::Char('s') | KeyCode::F(11) => self.resume(ResumeMode::Step, false)?,
            KeyCode::Char('T') => self.resume(ResumeMode::Step, true)?,
            KeyCode::Char('n') | KeyCode::F(10) => self.resume(ResumeMode::Next, false)?,
            KeyCode::Char('o') => self.resume(ResumeMode::Out, false)?,
            KeyCode::Char('c') | KeyCode::F(5) => self.resume(ResumeMode::Continue, false)?,
            KeyCode::Char('S') => self.skip_current_command()?,
            KeyCode::Char('r') => self.restart()?,
            KeyCode::Char('p') => self.export_markdown_report()?,
            KeyCode::Char('d') => self.capture_fs_snapshot(None)?,
            KeyCode::Char('D') => {
                self.prompt_mode = PromptMode::SnapshotScope;
                self.prompt_buf = self
                    .snapshot_scope
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
            }
            KeyCode::Char('v') => {
                self.bottom_tab = match self.bottom_tab {
                    BottomTab::Console => BottomTab::Events,
                    BottomTab::Events => BottomTab::Snapshots,
                    BottomTab::Snapshots => BottomTab::Console,
                };
                self.output_scroll = 0;
            }
            KeyCode::Char('t') => {
                self.theme_mode = self.theme_mode.next();
                self.status = format!("Switched visual theme to: {}", self.theme_mode.label());
            }
            KeyCode::Char('M') => {
                self.redact_secrets = !self.redact_secrets;
                self.status = format!(
                    "Secret redaction: {}",
                    if self.redact_secrets {
                        "ON (masking credentials)"
                    } else {
                        "OFF (showing raw values)"
                    }
                );
            }
            KeyCode::Char('h') => {
                self.show_system_env = !self.show_system_env;
                self.var_scroll = 0;
                self.status = format!(
                    "System [env] variables: {}",
                    if self.show_system_env {
                        "VISIBLE"
                    } else {
                        "HIDDEN (showing script vars only)"
                    }
                );
            }
            KeyCode::Char('[') | KeyCode::Left => {
                if self.proc_table.active_history_mut().step_back() {
                    if let Some(snap) = self.active_history().and_then(|h| h.active()).cloned() {
                        self.viewed_file = snap.file;
                        self.cursor_line = snap.line;
                        self.status =
                            format!("Time-travel: inspecting step #{}", snap.step_index);
                    }
                }
            }
            KeyCode::Char(']') | KeyCode::Right => {
                if self.proc_table.active_history_mut().step_forward() {
                    if let Some(snap) = self.active_history().and_then(|h| h.active()).cloned() {
                        self.viewed_file = snap.file;
                        self.cursor_line = snap.line;
                    }
                    let is_tt = self
                        .active_history()
                        .map(|h| h.is_time_traveling())
                        .unwrap_or(false);
                    if !is_tt {
                        self.status = "Returned to live execution state".into();
                    }
                }
            }
            KeyCode::Char('b') => self.toggle_breakpoint_at(self.cursor_line),
            KeyCode::Char('B') => {
                self.breakpoints.clear();
                self.cond_breakpoint.clear();
                self.status = "Cleared all line and conditional breakpoints".into();
            }
            KeyCode::Char('C') => {
                self.prompt_mode = PromptMode::CondBreak;
                self.prompt_buf = self.cond_breakpoint.clone();
            }
            KeyCode::Char('x') => {
                self.break_on_err = !self.break_on_err;
                self.status = format!(
                    "Break on error ($? != 0): {}",
                    if self.break_on_err { "ON" } else { "OFF" }
                );
            }
            KeyCode::Char(':') | KeyCode::Char('e') => {
                if self.exec_state == ExecState::Paused {
                    self.prompt_mode = PromptMode::Eval;
                    self.prompt_buf.clear();
                } else {
                    self.status = "Live eval is only available while PAUSED".into();
                }
            }
            KeyCode::Char('i') => {
                self.prompt_mode = PromptMode::SendStdin;
                self.prompt_buf.clear();
            }
            KeyCode::Char('/') => {
                self.prompt_mode = PromptMode::Filter;
                self.prompt_buf = self.var_filter.clone();
            }
            KeyCode::Char('w') => {
                self.prompt_mode = PromptMode::Watch;
                self.prompt_buf.clear();
            }
            KeyCode::Char('g') => {
                self.prompt_mode = PromptMode::GotoLine;
                self.prompt_buf.clear();
            }
            KeyCode::Char('f') => {
                self.prompt_mode = PromptMode::FindSource;
                self.prompt_buf = self.source_query.clone();
            }
            KeyCode::Char('N') => {
                self.find_next_in_source(self.cursor_line);
            }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    PaneFocus::Source => PaneFocus::Variables,
                    PaneFocus::Variables => PaneFocus::Stack,
                    PaneFocus::Stack => PaneFocus::Output,
                    PaneFocus::Output => PaneFocus::Source,
                };
            }
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(1),
            KeyCode::PageUp => self.scroll_up(12),
            KeyCode::PageDown => self.scroll_down(12),
            _ => {}
        }

        Ok(false)
    }

    fn handle_prompt_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Esc => {
                self.prompt_mode = PromptMode::None;
                self.prompt_buf.clear();
            }
            KeyCode::Backspace => {
                self.prompt_buf.pop();
                if self.prompt_mode == PromptMode::Filter {
                    self.var_filter = self.prompt_buf.clone();
                    self.var_scroll = 0;
                }
            }
            KeyCode::Enter => {
                let input = self.prompt_buf.trim().to_string();
                let pid = self.pid();
                match self.prompt_mode {
                    PromptMode::Eval if !input.is_empty() => {
                        self.logs.push(LogStream::Eval, format!("eval> {}", input));
                        match &mut self.backend {
                            SessionBackend::Local(s) => s.send_eval(&input)?,
                            SessionBackend::Remote(r) => {
                                r.send(&ClientMessage::Eval {
                                    pid,
                                    expr: input.clone(),
                                })?
                            }
                        }
                        self.status = format!("Evaluated: {}", input);
                    }
                    PromptMode::SendStdin => {
                        self.logs
                            .push(LogStream::Stdin, format!("stdin> {}", input));
                        match &mut self.backend {
                            SessionBackend::Local(s) => s.send_stdin(&input)?,
                            SessionBackend::Remote(r) => {
                                r.send(&ClientMessage::SendStdin {
                                    input: input.clone(),
                                })?
                            }
                        }
                        self.output_scroll = self.logs.len().saturating_sub(1);
                        self.status = "Forwarded line to script stdin".into();
                    }
                    PromptMode::CondBreak => {
                        self.cond_breakpoint = input.clone();
                        if input.is_empty() {
                            self.status = "Cleared conditional breakpoint".into();
                        } else {
                            self.status = format!("Set conditional trap: {}", input);
                        }
                    }
                    PromptMode::Filter => {
                        self.var_filter = input;
                        self.var_scroll = 0;
                    }
                    PromptMode::Watch if !input.is_empty() => {
                        if let Some(pos) = self.watches.iter().position(|x| x == &input) {
                            self.watches.remove(pos);
                            self.status = format!("Unpinned watch '{}'", input);
                        } else {
                            self.watches.push(input.clone());
                            self.status = format!("Pinned watch '{}'", input);
                        }
                    }
                    PromptMode::GotoLine => {
                        if let Ok(target) = input.parse::<usize>() {
                            let max = self.active_source_lines().len().max(1);
                            self.cursor_line = target.clamp(1, max);
                            self.status = format!("Jumped to line {}", self.cursor_line);
                        }
                    }
                    PromptMode::FindSource => {
                        self.source_query = input;
                        let start = self.cursor_line.saturating_sub(1);
                        self.find_next_in_source(start);
                    }
                    PromptMode::SnapshotScope if !input.is_empty() => {
                        self.capture_fs_snapshot(Some(PathBuf::from(input)))?;
                    }
                    _ => {}
                }
                self.prompt_mode = PromptMode::None;
                self.prompt_buf.clear();
            }
            KeyCode::Char(c) => {
                self.prompt_buf.push(c);
                if self.prompt_mode == PromptMode::Filter {
                    self.var_filter = self.prompt_buf.clone();
                    self.var_scroll = 0;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn scroll_up(&mut self, step: usize) {
        match self.focus {
            PaneFocus::Source => self.cursor_line = self.cursor_line.saturating_sub(step).max(1),
            PaneFocus::Variables => self.var_scroll = self.var_scroll.saturating_sub(step),
            PaneFocus::Stack => self.stack_scroll = self.stack_scroll.saturating_sub(step),
            PaneFocus::Output => self.output_scroll = self.output_scroll.saturating_sub(step),
        }
    }

    fn scroll_down(&mut self, step: usize) {
        match self.focus {
            PaneFocus::Source => {
                let max = self.active_source_lines().len().max(1);
                self.cursor_line = (self.cursor_line + step).min(max);
            }
            PaneFocus::Variables => {
                let max = self.filtered_vars().len().saturating_sub(1);
                self.var_scroll = (self.var_scroll + step).min(max);
            }
            PaneFocus::Stack => {
                let max = self
                    .active_history()
                    .and_then(|h| h.active())
                    .map(|s| s.stack.len().saturating_sub(1))
                    .unwrap_or(0);
                self.stack_scroll = (self.stack_scroll + step).min(max);
            }
            PaneFocus::Output => {
                let len = match self.bottom_tab {
                    BottomTab::Console => self.logs.len(),
                    BottomTab::Events => self.events.len(),
                    BottomTab::Snapshots => self
                        .latest_diff
                        .as_ref()
                        .map(|d| d.changes.len())
                        .unwrap_or(self.fs_snapshots.len()),
                };
                let max = len.saturating_sub(1);
                self.output_scroll = (self.output_scroll + step).min(max);
            }
        }
    }
}
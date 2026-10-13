// https://github.com/monji024

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};

use crate::engine::{parser::RawFrame, EngineEvent, ResumeMode, Session};
use crate::state::{
    history::{HistoryBuffer, Snapshot},
    BoundedLog, LogStream, SourceCache, VarEntry, VarKind,
};

const MAX_LOG_LINES: usize = 5_000;
const MAX_HISTORY_FRAMES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecState {
    Running,
    Paused,
    Exited(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneFocus {
    Source,
    Variables,
    Stack,
    Output,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayModal {
    None,
    InspectVar,
    Help,
}

pub enum TtyAction {
    None,
    Acquire(String),
    Release,
}

pub struct App {
    pub entry_script: PathBuf,
    pub script_args: Vec<String>,
    pub viewed_file: PathBuf,
    pub sources: SourceCache,
    pub session: Session,
    pub history: HistoryBuffer,
    pub logs: BoundedLog,
    pub breakpoints: HashSet<(PathBuf, usize)>,
    pub cond_breakpoint: String,
    pub break_on_err: bool,
    pub show_system_env: bool,
    pub watches: Vec<String>,
    pub var_filter: String,
    pub source_query: String,
    pub exec_state: ExecState,
    pub focus: PaneFocus,
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

        Ok(Self {
            viewed_file: entry_script.clone(),
            entry_script,
            script_args,
            sources,
            session,
            history: HistoryBuffer::new(MAX_HISTORY_FRAMES),
            logs: BoundedLog::new(MAX_LOG_LINES),
            breakpoints,
            cond_breakpoint: String::new(),
            break_on_err,
            show_system_env: false,
            watches: Vec::new(),
            var_filter: String::new(),
            source_query: String::new(),
            exec_state: ExecState::Running,
            focus: PaneFocus::Source,
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

    pub fn pid(&self) -> u32 {
        self.session.pid()
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

        while let Some(ev) = self.session.try_recv() {
            match ev {
                EngineEvent::Frame(raw) => {
                    if self.tty_passthrough {
                        self.tty_passthrough = false;
                        action = TtyAction::Release;
                    }
                    self.ingest_frame(raw);
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
                    self.output_scroll = self.logs.len().saturating_sub(1);
                }
                EngineEvent::Stderr(line) => {
                    self.logs.push(LogStream::Stderr, format!("[stderr] {}", line));
                    self.output_scroll = self.logs.len().saturating_sub(1);
                }
            }
        }

        if !matches!(self.exec_state, ExecState::Exited(_)) {
            if let Some(code) = self.session.try_wait_exit()? {
                if self.tty_passthrough {
                    self.tty_passthrough = false;
                    action = TtyAction::Release;
                }
                self.exec_state = ExecState::Exited(code);
                self.status = format!(
                    "Script finished with exit code {} ('r' restart, '[' time-travel, 'q' quit)",
                    code
                );
            }
        }

        Ok(action)
    }

    fn ingest_frame(&mut self, raw: RawFrame) {
        let canonical = std::fs::canonicalize(&raw.file).unwrap_or(raw.file);
        self.sources.ensure_loaded(&canonical);

        let latency = self.step_timer.elapsed();
        if !raw.is_sync {
            if let Some(prev) = self.history.active() {
                self.sources
                    .record_profile(prev.file.clone(), prev.line, latency);
            }
        }

        let prev_vars = self.history.active().map(|s| &s.vars);
        let mut vars = BTreeMap::new();

        for (name, (kind, value)) in raw.vars {
            let prev_val = prev_vars.and_then(|m| m.get(&name)).map(|old| old.value.clone());
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
        self.history.push(snapshot);
        self.exec_state = ExecState::Paused;

        if !raw.is_sync {
            let sub_badge = if raw.subshell > 0 {
                format!(" [subshell #{}]", raw.subshell)
            } else {
                String::new()
            };
            self.status = format!("line {}{} → {}", clamped_line, sub_badge, raw.cmd);
        }
    }

    pub fn resume(&mut self, mode: ResumeMode, force_tty: bool) -> Result<()> {
        if self.exec_state != ExecState::Paused {
            return Ok(());
        }
        self.history.return_to_live();
        let depth = self
            .history
            .active()
            .map(|s| s.stack.len())
            .unwrap_or(0);
        let bps: Vec<_> = self.breakpoints.iter().cloned().collect();

        self.step_timer = Instant::now();
        self.exec_state = ExecState::Running;
        self.session.send_resume(
            mode,
            depth,
            self.break_on_err,
            &bps,
            &self.cond_breakpoint,
            force_tty,
        )?;
        Ok(())
    }

    pub fn skip_current_command(&mut self) -> Result<()> {
        if self.exec_state != ExecState::Paused {
            return Ok(());
        }
        self.history.return_to_live();
        self.step_timer = Instant::now();
        self.exec_state = ExecState::Running;
        self.session.send_skip()?;
        self.status = "Skipped execution of line (extdebug dry-run)".into();
        Ok(())
    }

    pub fn restart(&mut self) -> Result<()> {
        self.session.shutdown();
        self.history.clear();
        self.logs.clear();
        self.sources.clear_profiles();
        self.step_counter = 0;
        self.tty_passthrough = false;
        self.step_timer = Instant::now();

        let bps: Vec<_> = self.breakpoints.iter().cloned().collect();
        self.session = Session::spawn(
            &self.entry_script,
            &self.script_args,
            &bps,
            self.break_on_err,
        )?;
        self.exec_state = ExecState::Running;
        self.status = "Session restarted cleanly".into();
        Ok(())
    }

    pub fn export_markdown_report(&mut self) -> Result<()> {
        let Some(snap) = self.history.active() else {
            self.status = "No snapshot available to export".into();
            return Ok(());
        };

        let mut report = String::new();
        let _ = writeln!(report, "# peek Diagnostic Snapshot");
        let _ = writeln!(report, "- **Script:** `{}`", snap.file.display());
        let _ = writeln!(report, "- **Line:** `{}` (`{}`)", snap.line, snap.cmd);
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
            let _ = writeln!(
                report,
                "- `{}` [{}]{} = `{:?}`",
                v.name,
                v.kind.tag(),
                delta,
                v.value
            );
        }

        std::fs::write("peek-report.md", report)?;
        self.status = "Exported diagnostic snapshot to ./peek-report.md".into();
        Ok(())
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
        let Some(snap) = self.history.active() else {
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
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?')
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
                self.session.shutdown();
                return Ok(true);
            }
            KeyCode::Char('?') => self.modal = OverlayModal::Help,
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
                if self.history.step_back() {
                    if let Some(snap) = self.history.active() {
                        self.viewed_file = snap.file.clone();
                        self.cursor_line = snap.line;
                        self.status =
                            format!("Time-travel: inspecting step #{}", snap.step_index);
                    }
                }
            }
            KeyCode::Char(']') | KeyCode::Right => {
                if self.history.step_forward() {
                    if let Some(snap) = self.history.active() {
                        self.viewed_file = snap.file.clone();
                        self.cursor_line = snap.line;
                    }
                    if !self.history.is_time_traveling() {
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
                match self.prompt_mode {
                    PromptMode::Eval if !input.is_empty() => {
                        self.logs.push(LogStream::Eval, format!("eval> {}", input));
                        self.session.send_eval(&input)?;
                        self.status = format!("Evaluated: {}", input);
                    }
                    PromptMode::SendStdin => {
                        self.logs.push(LogStream::Stdin, format!("stdin> {}", input));
                        self.session.send_stdin(&input)?;
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
                    .history
                    .active()
                    .map(|s| s.stack.len().saturating_sub(1))
                    .unwrap_or(0);
                self.stack_scroll = (self.stack_scroll + step).min(max);
            }
            PaneFocus::Output => {
                let max = self.logs.len().saturating_sub(1);
                self.output_scroll = (self.output_scroll + step).min(max);
            }
        }
    }
}
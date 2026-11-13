// https://github.com/monji024

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, BottomTab, ExecState, PaneFocus, PromptMode};
use crate::protocol::ProcessStatus;
use crate::snapshot::DiffKind;
use crate::state::{LogStream, VarKind};
use super::theme::Palette;

const BASH_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "until", "case",
    "esac", "function", "local", "declare", "readonly", "export", "return", "exit", "source",
    "shopt", "set", "unset", "trap", "eval", "exec",
];

pub fn render_minimal_viewport(f: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    let msg = format!(
        "peek [{}] L:{} PID:{} — {}",
        app.session_id,
        app.cursor_line,
        app.pid(),
        app.status
    );
    f.render_widget(
        Paragraph::new(msg)
            .style(Style::default().fg(pal.syn_text).bg(pal.bg_status_top))
            .wrap(Wrap { trim: true }),
        area,
    );
}

pub fn render_top_ribbon(f: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    let hist = app.active_history();
    let snap = hist.and_then(|h| h.active());
    let rc = snap.map(|s| s.exit_code).unwrap_or(0);
    let subshell = snap.map(|s| s.subshell).unwrap_or(0);

    let pipe_str = snap
        .filter(|s| s.pipe_status.len() > 1)
        .map(|s| {
            let parts: Vec<String> = s.pipe_status.iter().map(|c| c.to_string()).collect();
            format!(" pipe:[{}]", parts.join("|"))
        })
        .unwrap_or_default();

    let latency_str = snap
        .map(|s| format_duration_us(s.latency.as_micros()))
        .unwrap_or_else(|| "-".into());

    let frame_str = hist
        .and_then(|h| h.position_badge())
        .map(|(c, t)| format!("{}/{}", c, t))
        .unwrap_or_else(|| "0/0".into());

    let mut left_spans = vec![
        Span::styled(
            " peek ",
            Style::default()
                .fg(pal.badge_fg)
                .bg(pal.badge_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", app.theme_mode.label()),
            Style::default().fg(pal.syn_comment),
        ),
        Span::raw(" "),
        Span::styled("target ", Style::default().fg(pal.syn_comment)),
        Span::styled(
            app.viewed_file
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("script.sh"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("status ", Style::default().fg(pal.syn_comment)),
        Span::styled(
            format!("$?={}{}", rc, pipe_str),
            Style::default()
                .fg(if rc == 0 {
                    pal.syn_string
                } else {
                    Color::LightRed
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("step ", Style::default().fg(pal.syn_comment)),
        Span::styled(frame_str, Style::default().fg(Color::White)),
        Span::raw("  "),
        Span::styled("dt ", Style::default().fg(pal.syn_comment)),
        Span::styled(latency_str, Style::default().fg(pal.syn_text)),
        Span::raw("  "),
        Span::styled("procs ", Style::default().fg(pal.syn_comment)),
        Span::styled(
            format!("{} (sub:{})", app.proc_table.len(), subshell),
            Style::default().fg(Color::LightMagenta),
        ),
        Span::raw("  "),
        Span::styled("err-trap ", Style::default().fg(pal.syn_comment)),
        Span::styled(
            if app.break_on_err { "on" } else { "off" },
            Style::default()
                .fg(if app.break_on_err {
                    Color::LightRed
                } else {
                    pal.syn_comment
                })
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if !app.cond_breakpoint.is_empty() {
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled("when ", Style::default().fg(pal.syn_comment)));
        left_spans.push(Span::styled(
            app.cond_breakpoint.as_str(),
            Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    f.render_widget(
        Paragraph::new(Line::from(left_spans)).style(Style::default().bg(pal.bg_status_top)),
        area,
    );
}

pub fn render_source(f: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    let lines = app.active_source_lines();
    let focused = app.focus == PaneFocus::Source;
    let file_label = app
        .viewed_file
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("script.sh");

    let hist = app.active_history();
    let snap = hist.and_then(|h| h.active());
    let active_line = snap
        .filter(|s| s.file == app.viewed_file)
        .map(|s| s.line);

    let search_badge = if app.source_query.is_empty() {
        String::new()
    } else {
        format!(" ?\"{}\"", app.source_query)
    };

    let title = Line::from(vec![
        Span::styled(format!(" {} ", file_label), pal.title(focused)),
        Span::styled(
            format!(
                "({} lines, {} bp{}) ",
                lines.len(),
                app.breakpoints.len(),
                search_badge
            ),
            Style::default().fg(pal.syn_comment),
        ),
    ]);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(app.theme_mode.border_type())
        .border_style(pal.border(focused));

    let visible = area.height.saturating_sub(2) as usize;
    let is_traveling = hist.map(|h| h.is_time_traveling()).unwrap_or(false);

    let items: Vec<ListItem> = lines
        .iter()
        .enumerate()
        .skip(app.source_scroll)
        .take(visible)
        .map(|(idx, text)| {
            let lno = idx + 1;
            let has_bp = app.has_breakpoint(&app.viewed_file, lno);
            let is_exec = active_line == Some(lno);
            let is_cursor = app.cursor_line == lno;

            let bg = if is_exec {
                if is_traveling {
                    Style::default().bg(pal.bg_history_line)
                } else {
                    Style::default().bg(pal.bg_exec_line)
                }
            } else if is_cursor {
                Style::default().bg(pal.bg_cursor_line)
            } else {
                Style::default()
            };

            let heat_span = match app.sources.get_profile(&app.viewed_file, lno) {
                Some(prof) if prof.hits > 0 => {
                    let label = format!("{:>5} ", format_duration_us(prof.total_us));
                    let color = if prof.total_us >= 25_000 {
                        Color::LightRed
                    } else if prof.total_us >= 5_000 {
                        Color::Yellow
                    } else {
                        pal.syn_comment
                    };
                    Span::styled(label, bg.fg(color))
                }
                _ => Span::styled("      ", bg),
            };

            let mut spans = vec![
                Span::styled(
                    if has_bp { " *" } else { "  " },
                    bg.fg(Color::LightRed).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if is_exec { "> " } else { "  " },
                    bg.fg(if is_traveling {
                        Color::LightYellow
                    } else {
                        Color::LightGreen
                    })
                    .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:>4} ", lno),
                    bg.fg(if is_cursor {
                        Color::White
                    } else {
                        pal.syn_comment
                    }),
                ),
                heat_span,
                Span::styled("| ", bg.fg(pal.border_inactive)),
            ];

            spans.extend(tokenize_bash_line(text, bg, is_exec, &pal));
            ListItem::new(Line::from(spans)).style(bg)
        })
        .collect();

    f.render_widget(List::new(items).block(block), area);
}

fn tokenize_bash_line<'a>(
    line: &'a str,
    base_bg: Style,
    is_exec: bool,
    pal: &Palette,
) -> Vec<Span<'a>> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return vec![Span::styled(line, base_bg.fg(pal.syn_comment))];
    }

    let bold_mod = if is_exec {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };

    let mut spans = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i] as char;

        if ch == '#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            spans.push(Span::styled(&line[i..], base_bg.fg(pal.syn_comment)));
            break;
        }

        if ch == '"' || ch == '\'' {
            let quote = ch;
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] as char == '\\' && quote == '"' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if bytes[i] as char == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            spans.push(Span::styled(
                &line[start..i],
                base_bg.fg(pal.syn_string).add_modifier(bold_mod),
            ));
            continue;
        }

        if ch == '$' {
            let start = i;
            i += 1;
            if i < bytes.len() && (bytes[i] == b'{' || bytes[i] == b'(') {
                let close = if bytes[i] == b'{' { b'}' } else { b')' };
                while i < bytes.len() && bytes[i] != close {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'_'
                        || bytes[i] == b'?'
                        || bytes[i] == b'@')
                {
                    i += 1;
                }
            }
            spans.push(Span::styled(
                &line[start..i],
                base_bg.fg(pal.syn_var).add_modifier(bold_mod),
            ));
            continue;
        }

        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &line[start..i];
            let color = if BASH_KEYWORDS.contains(&word) {
                pal.syn_keyword
            } else if i < bytes.len() && bytes[i] == b'(' {
                Color::Yellow
            } else {
                pal.syn_text
            };
            spans.push(Span::styled(word, base_bg.fg(color).add_modifier(bold_mod)));
            continue;
        }

        let start = i;
        i += line[start..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        spans.push(Span::styled(
            &line[start..i],
            base_bg.fg(pal.syn_text).add_modifier(bold_mod),
        ));
    }

    spans
}

pub fn render_variables(f: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    let focused = app.focus == PaneFocus::Variables;
    let vars = app.filtered_vars();
    let changed_count = vars.iter().filter(|v| v.changed).count();

    let env_badge = if app.show_system_env { "all" } else { "script" };
    let redact_badge = if app.redact_secrets { "" } else { " [raw]" };
    let filter_str = if app.var_filter.is_empty() {
        String::new()
    } else {
        format!(" /{}", app.var_filter)
    };

    let title = Line::from(vec![
        Span::styled(" Locals & State ", pal.title(focused)),
        Span::styled(
            format!(
                "({} [{}]{}, {} Δ{}) ",
                vars.len(),
                env_badge,
                redact_badge,
                changed_count,
                filter_str
            ),
            Style::default().fg(pal.syn_comment),
        ),
    ]);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(app.theme_mode.border_type())
        .border_style(pal.border(focused));

    let visible = area.height.saturating_sub(2) as usize;
    let mut items = Vec::new();
    let active_vars = app
        .active_history()
        .and_then(|h| h.active())
        .map(|s| &s.vars);

    for watch in &app.watches {
        let val = active_vars
            .and_then(|m| m.get(watch))
            .map(|v| format_preview(&app.display_var_value(&v.name, &v.value)))
            .unwrap_or_else(|| "<unset>".into());

        let line = Line::from(vec![
            Span::styled(" * ", Style::default().fg(Color::LightYellow)),
            Span::styled("watch ", Style::default().fg(Color::Yellow)),
            Span::styled(
                watch.as_str(),
                Style::default()
                    .fg(Color::LightYellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" = ", Style::default().fg(pal.syn_comment)),
            Span::styled(val, Style::default().fg(Color::White)),
        ]);
        items.push(ListItem::new(line).style(Style::default().bg(pal.bg_watch_row)));
    }

    let remaining = visible.saturating_sub(items.len());
    for (idx, v) in vars
        .into_iter()
        .enumerate()
        .skip(app.var_scroll)
        .take(remaining)
    {
        let is_selected = focused && idx == app.var_scroll;
        let row_bg = if is_selected {
            Style::default().bg(pal.bg_cursor_line)
        } else {
            Style::default()
        };

        let tag_color = match v.kind {
            VarKind::Scalar => pal.syn_comment,
            VarKind::Exported => Color::Cyan,
            VarKind::ReadOnly => Color::LightRed,
            VarKind::Integer => Color::LightBlue,
            VarKind::Array => Color::LightMagenta,
            VarKind::AssocArray => Color::Yellow,
        };

        let shown = app.display_var_value(&v.name, &v.value);

        let line = Line::from(vec![
            Span::styled(
                if v.changed { " ~ " } else { "   " },
                row_bg
                    .fg(Color::LightMagenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{} ", v.kind.tag()), row_bg.fg(tag_color)),
            Span::styled(
                v.name.as_str(),
                if v.changed {
                    row_bg.fg(Color::LightYellow).add_modifier(Modifier::BOLD)
                } else {
                    row_bg.fg(pal.syn_text)
                },
            ),
            Span::styled(" = ", row_bg.fg(pal.syn_comment)),
            Span::styled(
                format_preview(&shown),
                if v.changed {
                    row_bg.fg(Color::LightGreen).add_modifier(Modifier::BOLD)
                } else {
                    row_bg.fg(pal.syn_string)
                },
            ),
        ]);
        items.push(ListItem::new(line).style(row_bg));
    }

    if items.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  No matching variables in current scope ('h' to toggle system env)",
            Style::default().fg(pal.syn_comment),
        ))));
    }

    f.render_widget(List::new(items).block(block), area);
}

fn format_preview(val: &str) -> String {
    let single_line = val.replace('\n', " \\n ");
    format!("\"{}\"", single_line)
}

pub fn render_stack(f: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    let focused = app.focus == PaneFocus::Stack;
    let snap = app.active_history().and_then(|h| h.active());
    let stack = snap.map(|s| s.stack.as_slice()).unwrap_or(&[]);
    let pwd = snap.map(|s| s.pwd.as_str()).unwrap_or(".");

    let title = Line::from(vec![
        Span::styled(" Call Stack & Procs ", pal.title(focused)),
        Span::styled(
            format!("(PID:{} {}) ", app.pid(), pwd),
            Style::default().fg(pal.syn_comment),
        ),
    ]);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(app.theme_mode.border_type())
        .border_style(pal.border(focused));

    let mut items = Vec::new();
    if stack.is_empty() {
        let line_no = snap.map(|s| s.line).unwrap_or(0);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(" 0 ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "<script-root>",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" :{}", line_no),
                Style::default().fg(pal.syn_comment),
            ),
        ])));
    } else {
        for (i, frame) in stack.iter().enumerate().skip(app.stack_scroll) {
            let fname = frame
                .file
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("?");
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", i), Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("{}()", frame.func),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {}:{}", fname, frame.line),
                    Style::default().fg(pal.syn_comment),
                ),
            ])));
        }
    }

    f.render_widget(List::new(items).block(block), area);
}

pub fn render_output(f: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    let focused = app.focus == PaneFocus::Output;

    let tab_label = match app.bottom_tab {
        BottomTab::Console => format!(" Console Output ({}) [v:events/snaps] ", app.logs.len()),
        BottomTab::Events => format!(" Event Timeline ({}) [v:snaps/console] ", app.events.len()),
        BottomTab::Snapshots => format!(
            " File Snapshots ({} snaps) [d:capture D:scope v:console] ",
            app.fs_snapshots.len()
        ),
    };

    let title = Line::from(vec![Span::styled(tab_label, pal.title(focused))]);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(app.theme_mode.border_type())
        .border_style(pal.border(focused));

    let visible = area.height.saturating_sub(2) as usize;

    let items: Vec<ListItem> = match app.bottom_tab {
        BottomTab::Console => {
            let start = app.output_scroll.saturating_add(1).saturating_sub(visible);
            app.logs
                .iter()
                .skip(start)
                .take(visible)
                .map(|entry| {
                    let style = match entry.stream {
                        LogStream::Stdout => Style::default().fg(pal.syn_text),
                        LogStream::Stderr => Style::default().fg(Color::LightRed),
                        LogStream::Eval => Style::default()
                            .fg(Color::LightCyan)
                            .add_modifier(Modifier::BOLD),
                        LogStream::Stdin => Style::default()
                            .fg(Color::LightYellow)
                            .add_modifier(Modifier::BOLD),
                        LogStream::Event => Style::default().fg(pal.syn_comment),
                    };
                    ListItem::new(Line::from(Span::styled(entry.text.as_str(), style)))
                })
                .collect()
        }
        BottomTab::Events => app
            .events
            .iter()
            .rev()
            .skip(app.output_scroll)
            .take(visible)
            .map(|ev| {
                let line = Line::from(vec![
                    Span::styled(
                        format!(" #{:<4} ", ev.seq),
                        Style::default().fg(pal.syn_comment),
                    ),
                    Span::styled(
                        format!("[PID {}] ", ev.pid),
                        Style::default().fg(Color::LightMagenta),
                    ),
                    Span::styled(
                        format!("{:<12} ", ev.kind.as_str()),
                        Style::default()
                            .fg(Color::LightCyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(ev.detail.as_str(), Style::default().fg(pal.syn_text)),
                ]);
                ListItem::new(line)
            })
            .collect(),
        BottomTab::Snapshots => {
            if let Some(ref diff) = app.latest_diff {
                if diff.changes.is_empty() {
                    vec![ListItem::new(Line::from(Span::styled(
                        format!(
                            "  Snapshot #{} vs #{}: no file changes detected in scope",
                            diff.from_id, diff.to_id
                        ),
                        Style::default().fg(pal.syn_comment),
                    )))]
                } else {
                    diff.changes
                        .iter()
                        .skip(app.output_scroll)
                        .take(visible)
                        .map(|ch| {
                            let color = match ch.kind {
                                DiffKind::Added => Color::LightGreen,
                                DiffKind::Modified => Color::LightYellow,
                                DiffKind::Deleted => Color::LightRed,
                            };
                            let sz = match (ch.old_size, ch.new_size) {
                                (None, Some(n)) => format!(" ({} B)", n),
                                (Some(o), Some(n)) => format!(" ({} B -> {} B)", o, n),
                                (Some(o), None) => format!(" (was {} B)", o),
                                _ => String::new(),
                            };
                            ListItem::new(Line::from(vec![
                                Span::styled(
                                    format!("  {} ", ch.kind.symbol()),
                                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(
                                    ch.rel_path.to_string_lossy().into_owned(),
                                    Style::default().fg(pal.syn_text),
                                ),
                                Span::styled(sz, Style::default().fg(pal.syn_comment)),
                            ]))
                        })
                        .collect()
                }
            } else if let Some(last) = app.fs_snapshots.last() {
                vec![ListItem::new(Line::from(Span::styled(
                    format!(
                        "  Baseline snapshot #{} captured ({} files in {}). Press 'd' again after stepping to compare.",
                        last.id,
                        last.entries.len(),
                        last.root_scope.display()
                    ),
                    Style::default().fg(pal.syn_text),
                )))]
            } else {
                vec![ListItem::new(Line::from(Span::styled(
                    "  No file snapshots captured yet. Press 'd' to snapshot working dir or 'D' to set scope.",
                    Style::default().fg(pal.syn_comment),
                )))]
            }
        }
    };

    f.render_widget(List::new(items).block(block), area);
}

pub fn render_statusline(f: &mut Frame, app: &App, area: Rect) {
    let pal = app.palette();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let is_tt = app
        .active_history()
        .map(|h| h.is_time_traveling())
        .unwrap_or(false);

    let (mode_str, mode_bg) = if app.prompt_mode != PromptMode::None {
        (" COMMAND ", Color::LightMagenta)
    } else if is_tt {
        (" HISTORY ", Color::Yellow)
    } else {
        match app.exec_state {
            ExecState::Paused => (" PAUSED ", Color::LightGreen),
            ExecState::Running => (" RUNNING ", Color::Cyan),
            ExecState::Exited(_) => (" EXITED ", Color::Blue),
            ExecState::Disconnected => (" DETACHED ", Color::LightRed),
        }
    };

    let status_row = Line::from(vec![
        Span::styled(
            mode_str,
            Style::default()
                .fg(Color::Black)
                .bg(mode_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(&app.status, Style::default().fg(pal.syn_text)),
        Span::raw("   "),
        Span::styled(
            format!(
                "L:{}/{}  PID:{}",
                app.cursor_line,
                app.active_source_lines().len(),
                app.pid()
            ),
            Style::default().fg(pal.syn_comment),
        ),
    ]);
    f.render_widget(
        Paragraph::new(status_row).style(Style::default().bg(pal.bg_status_top)),
        rows[0],
    );

    if app.prompt_mode != PromptMode::None {
        let prefix = match app.prompt_mode {
            PromptMode::Eval => ":eval ",
            PromptMode::Filter => "/filter ",
            PromptMode::Watch => "+watch ",
            PromptMode::GotoLine => ":goto ",
            PromptMode::FindSource => "?find ",
            PromptMode::SendStdin => "stdin> ",
            PromptMode::CondBreak => "when> ",
            PromptMode::SnapshotScope => "snap-dir> ",
            PromptMode::None => "",
        };
        let prompt_line = Line::from(vec![
            Span::styled(
                prefix,
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(&app.prompt_buf, Style::default().fg(Color::White)),
            Span::styled("_", Style::default().fg(Color::LightCyan)),
            Span::styled(
                "  (Enter confirm, Esc cancel)",
                Style::default().fg(pal.syn_comment),
            ),
        ]);
        f.render_widget(
            Paragraph::new(prompt_line).style(Style::default().bg(pal.bg_status_bottom)),
            rows[1],
        );
        return;
    }

    let k = Style::default()
        .fg(pal.syn_var)
        .add_modifier(Modifier::BOLD);
    let d = Style::default().fg(pal.syn_comment);

    let hints = Line::from(vec![
        Span::styled(" s", k),
        Span::styled(" step ", d),
        Span::styled("n", k),
        Span::styled(" next ", d),
        Span::styled("o", k),
        Span::styled(" out ", d),
        Span::styled("c", k),
        Span::styled(" cont ", d),
        Span::styled("S", k),
        Span::styled(" skip ", d),
        Span::styled("[/]", k),
        Span::styled(" hist ", d),
        Span::styled("b/C", k),
        Span::styled(" brk ", d),
        Span::styled("d/v", k),
        Span::styled(" snap/view ", d),
        Span::styled("P", k),
        Span::styled(" procs ", d),
        Span::styled("t", k),
        Span::styled(" theme ", d),
        Span::styled("?", k),
        Span::styled(" help ", d),
        Span::styled("q", k),
        Span::styled(" quit", d),
    ]);
    f.render_widget(
        Paragraph::new(hints).style(Style::default().bg(pal.bg_status_bottom)),
        rows[1],
    );
}

pub fn render_inspector_modal(f: &mut Frame, app: &App) {
    let pal = app.palette();
    let Some(var) = app.selected_var() else {
        return;
    };

    let area = centered_rect(72, 66, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                format!(" Inspect: {} [{}] ", var.name, var.kind.tag()),
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "(Esc / Enter to close, 'M' toggles secret masking) ",
                Style::default().fg(pal.syn_comment),
            ),
        ]))
        .borders(Borders::ALL)
        .border_type(app.theme_mode.border_type())
        .border_style(Style::default().fg(pal.border_active))
        .style(Style::default().bg(pal.bg_modal));

    let mut lines: Vec<Line> = Vec::new();
    let cur_val = app.display_var_value(&var.name, &var.value);

    if let Some(prev) = &var.prev_value {
        if prev != &var.value {
            let prev_shown = app.display_var_value(&var.name, prev);
            lines.push(Line::from(vec![
                Span::styled(
                    " - old | ",
                    Style::default()
                        .fg(Color::LightRed)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    prev_shown.replace('\n', " \\n "),
                    Style::default().fg(Color::LightRed),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled(
                    " + new | ",
                    Style::default()
                        .fg(Color::LightGreen)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    cur_val.replace('\n', " \\n "),
                    Style::default().fg(Color::LightGreen),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                " ------+--------------------------------------------------------",
                Style::default().fg(pal.border_inactive),
            )));
        }
    }

    for (idx, raw_line) in cur_val.lines().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:>5} | ", idx + 1),
                Style::default().fg(pal.syn_comment),
            ),
            Span::styled(raw_line.to_string(), Style::default().fg(Color::White)),
        ]));
    }
    if cur_val.is_empty() {
        lines.push(Line::from(Span::styled(
            " <empty string>",
            Style::default().fg(pal.syn_comment),
        )));
    }

    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

pub fn render_processes_modal(f: &mut Frame, app: &App) {
    let pal = app.palette();
    let area = centered_rect(70, 56, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                format!(" Session '{}' — Tracked Bash Processes ", app.session_id),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "('P' cycle active process, Esc close) ",
                Style::default().fg(pal.syn_comment),
            ),
        ]))
        .borders(Borders::ALL)
        .border_type(app.theme_mode.border_type())
        .border_style(Style::default().fg(pal.border_active))
        .style(Style::default().bg(pal.bg_modal));

    let active_pid = app.pid();
    let lines: Vec<Line> = app
        .proc_table
        .list()
        .into_iter()
        .map(|p| {
            let marker = if p.pid == active_pid { " > " } else { "   " };
            let st_color = match p.status {
                ProcessStatus::Paused => Color::LightGreen,
                ProcessStatus::Running => Color::Cyan,
                ProcessStatus::Exited(_) => Color::Yellow,
                ProcessStatus::Disconnected => Color::LightRed,
            };
            Line::from(vec![
                Span::styled(
                    marker,
                    Style::default()
                        .fg(Color::LightYellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("PID {:<7} ", p.pid),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("(PPID {:<7}) ", p.ppid),
                    Style::default().fg(pal.syn_comment),
                ),
                Span::styled(
                    format!("[{:<12}] ", p.status.as_tag()),
                    Style::default().fg(st_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{}: {}", p.role, p.script.display()),
                    Style::default().fg(pal.syn_text),
                ),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(lines).block(block), area);
}

pub fn render_help_modal(f: &mut Frame, app: &App) {
    let pal = app.palette();
    let area = centered_rect(72, 80, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            " peek — Keyboard Reference (@monji024) ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )]))
        .borders(Borders::ALL)
        .border_type(app.theme_mode.border_type())
        .border_style(Style::default().fg(pal.border_active))
        .style(Style::default().bg(pal.bg_modal));

    let rows = vec![
        ("s / F11", "Step into next Bash command, sourced script, or subshell"),
        ("T (Shift+t)", "Step current command with foreground /dev/tty handoff"),
        ("n / F10", "Step over function call or command substitution"),
        ("o", "Step out of current function back to caller"),
        ("c / F5", "Continue execution until breakpoint, condition, or error"),
        ("S (Shift+s)", "Skip executing current line (extdebug dry-run)"),
        ("[ / ] or <- / ->", "Time-travel backward / forward through 256 frames"),
        ("b / B", "Toggle line breakpoint / Clear all breakpoints"),
        ("C (Shift+c)", "Set conditional breakpoint (e.g. (( i == 50 )))"),
        ("x", "Toggle automatic pause on non-zero exit code ($? != 0)"),
        ("P (Shift+p)", "Cycle active Bash process and inspect process tree"),
        ("d / D", "Capture directory snapshot & diff / Set snapshot scope"),
        ("v", "Switch bottom pane between Console, Events, and Snapshots"),
        ("t", "Cycle visual theme (Classic, Luna Blue, Terminal Native)"),
        ("M (Shift+m)", "Toggle automatic credential & secret value redaction"),
        ("h", "Toggle system environment variables ([env]) visibility"),
        ("Enter", "Open multiline value & old/new diff inspector modal"),
        (": / e", "Evaluate live Bash expression in current process context"),
        ("i", "Send a line of text to script standard input (read)"),
        ("/ and w", "Filter variables / Pin variable to watchpoint list"),
        ("f / N / g", "Find text in source / Next match / Goto line number"),
        ("p / r / q", "Export report / Restart session / Quit debugger"),
    ];

    let lines: Vec<Line> = rows
        .into_iter()
        .map(|(k, desc)| {
            Line::from(vec![
                Span::styled(
                    format!("  {:<18}", k),
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(desc, Style::default().fg(pal.syn_text)),
            ])
        })
        .collect();

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left),
        area,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}

fn format_duration_us(us: u128) -> String {
    if us >= 1_000_000 {
        format!("{:.1}s", us as f64 / 1_000_000.0)
    } else if us >= 1_000 {
        format!("{}ms", us / 1_000)
    } else {
        format!("{}µs", us)
    }
}
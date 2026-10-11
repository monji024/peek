// https://github.com/monji024

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, ExecState, PaneFocus, PromptMode};
use crate::state::{LogStream, VarKind};
use super::theme::Theme;

const BASH_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "until", "case",
    "esac", "function", "local", "declare", "readonly", "export", "return", "exit", "source",
    "shopt", "set", "unset", "trap", "eval", "exec",
];

pub fn render_top_ribbon(f: &mut Frame, app: &App, area: Rect) {
    let snap = app.history.active();
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

    let frame_str = app
        .history
        .position_badge()
        .map(|(c, t)| format!("{}/{}", c, t))
        .unwrap_or_else(|| "0/0".into());

    let mut left_spans = vec![
        Span::styled(
            " peek ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(110, 175, 255))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " v1.0.0 ",
            Style::default()
                .fg(Color::Rgb(170, 180, 205))
                .bg(Color::Rgb(32, 36, 50)),
        ),
        Span::raw("  "),
        Span::styled("target ", Style::default().fg(Color::DarkGray)),
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
        Span::styled("status ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("$?={}{}", rc, pipe_str),
            Style::default()
                .fg(if rc == 0 {
                    Color::Rgb(158, 206, 106)
                } else {
                    Color::LightRed
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("step ", Style::default().fg(Color::DarkGray)),
        Span::styled(frame_str, Style::default().fg(Color::White)),
        Span::raw("  "),
        Span::styled("dt ", Style::default().fg(Color::DarkGray)),
        Span::styled(latency_str, Style::default().fg(Color::Rgb(185, 192, 210))),
        Span::raw("  "),
        Span::styled("subshell ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{}", subshell), Style::default().fg(Color::Magenta)),
        Span::raw("  "),
        Span::styled("err-trap ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            if app.break_on_err { "on" } else { "off" },
            Style::default()
                .fg(if app.break_on_err {
                    Color::LightRed
                } else {
                    Color::DarkGray
                })
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if !app.cond_breakpoint.is_empty() {
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled("when ", Style::default().fg(Color::DarkGray)));
        left_spans.push(Span::styled(
            app.cond_breakpoint.as_str(),
            Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    f.render_widget(
        Paragraph::new(Line::from(left_spans)).style(Style::default().bg(Theme::BG_STATUS_TOP)),
        area,
    );
}

pub fn render_source(f: &mut Frame, app: &App, area: Rect) {
    let lines = app.active_source_lines();
    let focused = app.focus == PaneFocus::Source;
    let file_label = app
        .viewed_file
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("script.sh");

    let snap = app.history.active();
    let active_line = snap
        .filter(|s| s.file == app.viewed_file)
        .map(|s| s.line);

    let search_badge = if app.source_query.is_empty() {
        String::new()
    } else {
        format!(" ?\"{}\"", app.source_query)
    };

    let title = Line::from(vec![
        Span::styled(format!(" {} ", file_label), Theme::title(focused)),
        Span::styled(
            format!("({} lines, {} bp{}) ", lines.len(), app.breakpoints.len(), search_badge),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border(focused));

    let visible = area.height.saturating_sub(2) as usize;
    let is_traveling = app.history.is_time_traveling();

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
                    Style::default().bg(Theme::BG_HISTORY_LINE)
                } else {
                    Style::default().bg(Theme::BG_EXEC_LINE)
                }
            } else if is_cursor {
                Style::default().bg(Theme::BG_CURSOR_LINE)
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
                        Color::Rgb(85, 115, 95)
                    };
                    Span::styled(label, bg.fg(color))
                }
                _ => Span::styled("      ", bg),
            };

            let mut spans = vec![
                Span::styled(
                    if has_bp { " ●" } else { "  " },
                    bg.fg(Color::LightRed).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if is_exec { "▶ " } else { "  " },
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
                        Color::Rgb(88, 94, 114)
                    }),
                ),
                heat_span,
                Span::styled("│ ", bg.fg(Color::Rgb(54, 59, 74))),
            ];

            spans.extend(tokenize_bash_line(text, bg, is_exec));
            ListItem::new(Line::from(spans)).style(bg)
        })
        .collect();

    f.render_widget(List::new(items).block(block), area);
}

fn tokenize_bash_line(line: &str, base_bg: Style, is_exec: bool) -> Vec<Span<'_>> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return vec![Span::styled(line, base_bg.fg(Theme::SYN_COMMENT))];
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
            spans.push(Span::styled(&line[i..], base_bg.fg(Theme::SYN_COMMENT)));
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
                base_bg.fg(Theme::SYN_STRING).add_modifier(bold_mod),
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
                base_bg.fg(Theme::SYN_VAR).add_modifier(bold_mod),
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
                Theme::SYN_KEYWORD
            } else if i < bytes.len() && bytes[i] == b'(' {
                Color::Rgb(224, 175, 104)
            } else {
                Theme::SYN_TEXT
            };
            spans.push(Span::styled(word, base_bg.fg(color).add_modifier(bold_mod)));
            continue;
        }

        let start = i;
        i += line[start..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        spans.push(Span::styled(
            &line[start..i],
            base_bg.fg(Theme::SYN_TEXT).add_modifier(bold_mod),
        ));
    }

    spans
}

pub fn render_variables(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == PaneFocus::Variables;
    let vars = app.filtered_vars();
    let changed_count = vars.iter().filter(|v| v.changed).count();

    let env_badge = if app.show_system_env { "all" } else { "script" };
    let filter_str = if app.var_filter.is_empty() {
        String::new()
    } else {
        format!(" /{}", app.var_filter)
    };

    let title = Line::from(vec![
        Span::styled(" Locals & State ", Theme::title(focused)),
        Span::styled(
            format!("({} [{}], {} Δ{}) ", vars.len(), env_badge, changed_count, filter_str),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border(focused));

    let visible = area.height.saturating_sub(2) as usize;
    let mut items = Vec::new();
    let active_vars = app.history.active().map(|s| &s.vars);

    for watch in &app.watches {
        let val = active_vars
            .and_then(|m| m.get(watch))
            .map(|v| format_preview(&v.value))
            .unwrap_or_else(|| "<unset>".into());

        let line = Line::from(vec![
            Span::styled(" ★ ", Style::default().fg(Color::LightYellow)),
            Span::styled("watch ", Style::default().fg(Color::Yellow)),
            Span::styled(
                watch.as_str(),
                Style::default()
                    .fg(Color::LightYellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" = ", Style::default().fg(Color::DarkGray)),
            Span::styled(val, Style::default().fg(Color::White)),
        ]);
        items.push(ListItem::new(line).style(Style::default().bg(Theme::BG_WATCH_ROW)));
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
            Style::default().bg(Theme::BG_CURSOR_LINE)
        } else {
            Style::default()
        };

        let tag_color = match v.kind {
            VarKind::Scalar => Color::Rgb(110, 120, 145),
            VarKind::Exported => Color::Cyan,
            VarKind::ReadOnly => Color::LightRed,
            VarKind::Integer => Color::LightBlue,
            VarKind::Array => Color::LightMagenta,
            VarKind::AssocArray => Color::Yellow,
        };

        let line = Line::from(vec![
            Span::styled(
                if v.changed { " Δ " } else { "   " },
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
                    row_bg.fg(Color::Rgb(200, 208, 224))
                },
            ),
            Span::styled(" = ", row_bg.fg(Color::DarkGray)),
            Span::styled(
                format_preview(&v.value),
                if v.changed {
                    row_bg.fg(Color::LightGreen).add_modifier(Modifier::BOLD)
                } else {
                    row_bg.fg(Theme::SYN_STRING)
                },
            ),
        ]);
        items.push(ListItem::new(line).style(row_bg));
    }

    f.render_widget(List::new(items).block(block), area);
}

fn format_preview(val: &str) -> String {
    let single_line = val.replace('\n', " ↵ ");
    format!("\"{}\"", single_line)
}

pub fn render_stack(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == PaneFocus::Stack;
    let snap = app.history.active();
    let stack = snap.map(|s| s.stack.as_slice()).unwrap_or(&[]);
    let pwd = snap.map(|s| s.pwd.as_str()).unwrap_or(".");

    let title = Line::from(vec![
        Span::styled(" Call Stack ", Theme::title(focused)),
        Span::styled(format!("({}) ", pwd), Style::default().fg(Color::DarkGray)),
    ]);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border(focused));

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
                Style::default().fg(Color::DarkGray),
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
                        .fg(Color::Rgb(224, 175, 104))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {}:{}", fname, frame.line),
                    Style::default().fg(Color::DarkGray),
                ),
            ])));
        }
    }

    f.render_widget(List::new(items).block(block), area);
}

pub fn render_output(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == PaneFocus::Output;
    let title = Line::from(vec![
        Span::styled(" Console Output & Stdin ", Theme::title(focused)),
        Span::styled(
            format!("({} lines) ", app.logs.len()),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border(focused));

    let visible = area.height.saturating_sub(2) as usize;
    let start = app.output_scroll.saturating_add(1).saturating_sub(visible);

    let items: Vec<ListItem> = app
        .logs
        .iter()
        .skip(start)
        .take(visible)
        .map(|entry| {
            let style = match entry.stream {
                LogStream::Stdout => Style::default().fg(Color::Rgb(195, 202, 218)),
                LogStream::Stderr => Style::default().fg(Color::LightRed),
                LogStream::Eval => Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
                LogStream::Stdin => Style::default()
                    .fg(Color::LightYellow)
                    .add_modifier(Modifier::BOLD),
            };
            ListItem::new(Line::from(Span::styled(entry.text.as_str(), style)))
        })
        .collect();

    f.render_widget(List::new(items).block(block), area);
}

pub fn render_statusline(f: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let (mode_str, mode_bg) = if app.prompt_mode != PromptMode::None {
        (" COMMAND ", Color::LightMagenta)
    } else if app.history.is_time_traveling() {
        (" HISTORY ", Color::Rgb(224, 175, 104))
    } else {
        match app.exec_state {
            ExecState::Paused => (" PAUSED ", Color::Rgb(158, 206, 106)),
            ExecState::Running => (" RUNNING ", Color::Cyan),
            ExecState::Exited(_) => (" EXITED ", Color::Rgb(110, 130, 180)),
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
        Span::styled(&app.status, Style::default().fg(Color::Rgb(215, 220, 235))),
        Span::raw("   "),
        Span::styled(
            format!("L:{}/{}  PID:{}", app.cursor_line, app.active_source_lines().len(), app.pid()),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(
        Paragraph::new(status_row).style(Style::default().bg(Theme::BG_STATUS_TOP)),
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
            Span::styled("█", Style::default().fg(Color::LightCyan)),
            Span::styled("  (Enter confirm, Esc cancel)", Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(
            Paragraph::new(prompt_line).style(Style::default().bg(Theme::BG_STATUS_BOTTOM)),
            rows[1],
        );
        return;
    }

    let k = Style::default()
        .fg(Color::Rgb(125, 207, 255))
        .add_modifier(Modifier::BOLD);
    let d = Style::default().fg(Color::Rgb(130, 138, 158));

    let hints = Line::from(vec![
        Span::styled(" s", k), Span::styled(" step  ", d),
        Span::styled("n", k), Span::styled(" next  ", d),
        Span::styled("o", k), Span::styled(" out  ", d),
        Span::styled("c", k), Span::styled(" cont  ", d),
        Span::styled("S", k), Span::styled(" skip  ", d),
        Span::styled("[/]", k), Span::styled(" history  ", d),
        Span::styled("b/C", k), Span::styled(" brk/cond  ", d),
        Span::styled("i", k), Span::styled(" stdin  ", d),
        Span::styled(":", k), Span::styled(" eval  ", d),
        Span::styled("f/g", k), Span::styled(" find/goto  ", d),
        Span::styled("h", k), Span::styled(" env  ", d),
        Span::styled("?", k), Span::styled(" help  ", d),
        Span::styled("q", k), Span::styled(" quit", d),
    ]);
    f.render_widget(
        Paragraph::new(hints).style(Style::default().bg(Theme::BG_STATUS_BOTTOM)),
        rows[1],
    );
}

pub fn render_inspector_modal(f: &mut Frame, app: &App) {
    let Some(var) = app.selected_var() else {
        return;
    };

    let area = centered_rect(70, 64, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                format!(" Inspect: {} [{}] ", var.name, var.kind.tag()),
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("(Esc / Enter to close) ", Style::default().fg(Color::DarkGray)),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Theme::BORDER_ACTIVE))
        .style(Style::default().bg(Theme::BG_MODAL));

    let mut lines: Vec<Line> = Vec::new();

    if let Some(prev) = &var.prev_value {
        if prev != &var.value {
            lines.push(Line::from(vec![
                Span::styled(" - old │ ", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)),
                Span::styled(prev.replace('\n', " ↵ "), Style::default().fg(Color::LightRed)),
            ]));
            lines.push(Line::from(vec![
                Span::styled(" + new │ ", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
                Span::styled(var.value.replace('\n', " ↵ "), Style::default().fg(Color::LightGreen)),
            ]));
            lines.push(Line::from(Span::styled(
                " ──────┼────────────────────────────────────────────────────────",
                Style::default().fg(Color::Rgb(54, 59, 74)),
            )));
        }
    }

    for (idx, raw_line) in var.value.lines().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(format!(" {:>5} │ ", idx + 1), Style::default().fg(Color::DarkGray)),
            Span::styled(raw_line.to_string(), Style::default().fg(Color::White)),
        ]));
    }
    if var.value.is_empty() {
        lines.push(Line::from(Span::styled(
            " <empty string>",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

pub fn render_help_modal(f: &mut Frame) {
    let area = centered_rect(68, 74, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " peek v1.0.0 — Keyboard Reference (@monji024) ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Theme::BORDER_ACTIVE))
        .style(Style::default().bg(Theme::BG_MODAL));

    let rows = vec![
        ("s / F11", "Step into next Bash command or sourced script"),
        ("n / F10", "Step over function call at current stack depth"),
        ("o", "Step out of current function back to caller"),
        ("c / F5", "Continue at native speed until breakpoint or condition"),
        ("S (Shift+s)", "Skip executing current line (extdebug dry-run)"),
        ("r", "Restart debugging session keeping breakpoints/watches"),
        ("[ / ] or ← / →", "Time-travel backward / forward through 256 frames"),
        ("b / B", "Toggle line breakpoint / Clear all breakpoints"),
        ("C (Shift+c)", "Set native conditional breakpoint (e.g. (( i == 50 )))"),
        ("x", "Toggle automatic pause on non-zero exit code ($? != 0)"),
        ("i", "Send a line of text to child script's standard input (read)"),
        ("h", "Toggle system environment variables ([env]) visibility"),
        ("Enter", "Open multiline value & old/new diff inspector modal"),
        (": / e", "Evaluate live Bash expression in current process context"),
        ("/ and w", "Filter variables / Pin variable to watchpoint list"),
        ("f / N / g", "Find text in source / Next match / Goto line number"),
        ("p", "Export Markdown diagnostic report to peek-report.md"),
        ("Tab", "Cycle focus across Source, Variables, Stack, and Output"),
    ];

    let lines: Vec<Line> = rows
        .into_iter()
        .map(|(k, desc)| {
            Line::from(vec![
                Span::styled(
                    format!("  {:<16}", k),
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(desc, Style::default().fg(Color::Rgb(205, 210, 225))),
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
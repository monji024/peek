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

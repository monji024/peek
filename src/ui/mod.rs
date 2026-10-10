// https://github.com/monji024

pub mod theme;
pub mod widgets;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

use crate::app::{App, OverlayModal};

pub fn draw(f: &mut Frame, app: &mut App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(12),
            Constraint::Length(7),
            Constraint::Length(2),
        ])
        .split(f.area());

    widgets::render_top_ribbon(f, app, root[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(root[1]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(body[1]);

    let source_height = body[0].height.saturating_sub(2) as usize;
    app.sync_viewport(source_height);

    widgets::render_source(f, app, body[0]);
    widgets::render_variables(f, app, right[0]);
    widgets::render_stack(f, app, right[1]);
    widgets::render_output(f, app, root[2]);
    widgets::render_statusline(f, app, root[3]);

    match app.modal {
        OverlayModal::None => {}
        OverlayModal::InspectVar => widgets::render_inspector_modal(f, app),
        OverlayModal::Help => widgets::render_help_modal(f),
    }
}
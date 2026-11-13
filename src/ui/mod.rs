// https://github.com/monji024

pub mod theme;
pub mod widgets;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

use crate::app::{App, OverlayModal};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    if area.height < 6 || area.width < 24 {
        widgets::render_minimal_viewport(f, app, area);
        return;
    }

    let bottom_len = if area.height < 16 { 4 } else { 7 };
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(bottom_len),
            Constraint::Length(2),
        ])
        .split(area);

    widgets::render_top_ribbon(f, app, root[0]);

    if area.width < 70 {
        let source_height = root[1].height.saturating_sub(2) as usize;
        app.sync_viewport(source_height);
        widgets::render_source(f, app, root[1]);
    } else {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(root[1]);

        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(body[1]);

        let source_height = body[0].height.saturating_sub(2) as usize;
        app.sync_viewport(source_height);

        widgets::render_source(f, app, body[0]);
        widgets::render_variables(f, app, right[0]);
        widgets::render_stack(f, app, right[1]);
    }

    widgets::render_output(f, app, root[2]);
    widgets::render_statusline(f, app, root[3]);

    match app.modal {
        OverlayModal::None => {}
        OverlayModal::InspectVar => widgets::render_inspector_modal(f, app),
        OverlayModal::Help => widgets::render_help_modal(f, app),
        OverlayModal::Processes => widgets::render_processes_modal(f, app),
    }
}
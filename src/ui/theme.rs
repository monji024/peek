// https://github.com/monji024

use ratatui::style::{Color, Modifier, Style};

pub struct Theme;

impl Theme {
    pub const BG_STATUS_TOP: Color = Color::Rgb(22, 24, 33);
    pub const BG_STATUS_BOTTOM: Color = Color::Rgb(19, 21, 29);
    pub const BG_EXEC_LINE: Color = Color::Rgb(30, 56, 40);
    pub const BG_HISTORY_LINE: Color = Color::Rgb(62, 46, 22);
    pub const BG_CURSOR_LINE: Color = Color::Rgb(30, 33, 46);
    pub const BG_WATCH_ROW: Color = Color::Rgb(34, 30, 22);
    pub const BG_MODAL: Color = Color::Rgb(20, 22, 30);

    pub const BORDER_ACTIVE: Color = Color::Rgb(110, 175, 255);
    pub const BORDER_INACTIVE: Color = Color::Rgb(54, 59, 74);

    pub const SYN_COMMENT: Color = Color::Rgb(105, 112, 132);
    pub const SYN_KEYWORD: Color = Color::Rgb(198, 145, 255);
    pub const SYN_STRING: Color = Color::Rgb(158, 206, 106);
    pub const SYN_VAR: Color = Color::Rgb(125, 207, 255);
    pub const SYN_TEXT: Color = Color::Rgb(208, 214, 228);

    pub fn border(focused: bool) -> Style {
        if focused {
            Style::default().fg(Self::BORDER_ACTIVE)
        } else {
            Style::default().fg(Self::BORDER_INACTIVE)
        }
    }

    pub fn title(focused: bool) -> Style {
        if focused {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(145, 152, 170))
        }
    }
}
// https://github.com/monji024

use ratatui::{
    style::{Color, Modifier, Style},
    widgets::BorderType,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    TerminalNative,
    LunaBlue,
    Classic,
}

impl ThemeMode {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "luna" | "luna-blue" | "lunablue" | "xp" => ThemeMode::LunaBlue,
            "native" | "terminal" | "terminal-native" | "ansi" => ThemeMode::TerminalNative,
            _ => ThemeMode::Classic,
        }
    }

    pub fn next(self) -> Self {
        match self {
            ThemeMode::Classic => ThemeMode::LunaBlue,
            ThemeMode::LunaBlue => ThemeMode::TerminalNative,
            ThemeMode::TerminalNative => ThemeMode::Classic,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::TerminalNative => "Terminal Native",
            ThemeMode::LunaBlue => "Luna Blue",
            ThemeMode::Classic => "Classic",
        }
    }

    pub fn border_type(self) -> BorderType {
        match self {
            ThemeMode::TerminalNative => BorderType::Plain,
            ThemeMode::LunaBlue => BorderType::Double,
            ThemeMode::Classic => BorderType::Rounded,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    Ansi16,
    Ansi256,
    TrueColor,
}

impl ColorDepth {
    pub fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            return ColorDepth::Ansi16;
        }
        let colorterm = std::env::var("COLORTERM")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return ColorDepth::TrueColor;
        }
        let term = std::env::var("TERM")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if term.contains("256color") {
            ColorDepth::Ansi256
        } else {
            ColorDepth::Ansi16
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub mode: ThemeMode,
    pub bg_status_top: Color,
    pub bg_status_bottom: Color,
    pub bg_exec_line: Color,
    pub bg_history_line: Color,
    pub bg_cursor_line: Color,
    pub bg_watch_row: Color,
    pub bg_modal: Color,
    pub border_active: Color,
    pub border_inactive: Color,
    pub syn_comment: Color,
    pub syn_keyword: Color,
    pub syn_string: Color,
    pub syn_var: Color,
    pub syn_text: Color,
    pub badge_fg: Color,
    pub badge_bg: Color,
}

impl Palette {
    pub fn for_mode(mode: ThemeMode, depth: ColorDepth) -> Self {
        if mode == ThemeMode::TerminalNative || depth == ColorDepth::Ansi16 {
            return Self {
                mode,
                bg_status_top: Color::Reset,
                bg_status_bottom: Color::Reset,
                bg_exec_line: Color::DarkGray,
                bg_history_line: Color::Yellow,
                bg_cursor_line: Color::Reset,
                bg_watch_row: Color::Reset,
                bg_modal: Color::Black,
                border_active: Color::Cyan,
                border_inactive: Color::DarkGray,
                syn_comment: Color::DarkGray,
                syn_keyword: Color::Magenta,
                syn_string: Color::Green,
                syn_var: Color::Cyan,
                syn_text: Color::Reset,
                badge_fg: Color::Black,
                badge_bg: Color::Cyan,
            };
        }

        match mode {
            ThemeMode::LunaBlue => {
                if depth == ColorDepth::Ansi256 {
                    Self {
                        mode,
                        bg_status_top: Color::Indexed(26),
                        bg_status_bottom: Color::Indexed(19),
                        bg_exec_line: Color::Indexed(22),
                        bg_history_line: Color::Indexed(94),
                        bg_cursor_line: Color::Indexed(24),
                        bg_watch_row: Color::Indexed(18),
                        bg_modal: Color::Indexed(17),
                        border_active: Color::Indexed(75),
                        border_inactive: Color::Indexed(60),
                        syn_comment: Color::Indexed(103),
                        syn_keyword: Color::Indexed(221),
                        syn_string: Color::Indexed(120),
                        syn_var: Color::Indexed(117),
                        syn_text: Color::Indexed(255),
                        badge_fg: Color::White,
                        badge_bg: Color::Indexed(28),
                    }
                } else {
                    Self {
                        mode,
                        bg_status_top: Color::Rgb(0, 84, 227),
                        bg_status_bottom: Color::Rgb(16, 42, 96),
                        bg_exec_line: Color::Rgb(28, 82, 48),
                        bg_history_line: Color::Rgb(92, 66, 18),
                        bg_cursor_line: Color::Rgb(26, 56, 118),
                        bg_watch_row: Color::Rgb(22, 46, 92),
                        bg_modal: Color::Rgb(14, 30, 68),
                        border_active: Color::Rgb(92, 174, 255),
                        border_inactive: Color::Rgb(58, 92, 160),
                        syn_comment: Color::Rgb(142, 168, 214),
                        syn_keyword: Color::Rgb(255, 206, 84),
                        syn_string: Color::Rgb(140, 232, 132),
                        syn_var: Color::Rgb(130, 218, 255),
                        syn_text: Color::Rgb(240, 245, 255),
                        badge_fg: Color::White,
                        badge_bg: Color::Rgb(58, 166, 54),
                    }
                }
            }
            ThemeMode::Classic | ThemeMode::TerminalNative => {
                if depth == ColorDepth::Ansi256 {
                    Self {
                        mode,
                        bg_status_top: Color::Indexed(235),
                        bg_status_bottom: Color::Indexed(234),
                        bg_exec_line: Color::Indexed(22),
                        bg_history_line: Color::Indexed(58),
                        bg_cursor_line: Color::Indexed(236),
                        bg_watch_row: Color::Indexed(236),
                        bg_modal: Color::Indexed(235),
                        border_active: Color::Indexed(75),
                        border_inactive: Color::Indexed(240),
                        syn_comment: Color::Indexed(243),
                        syn_keyword: Color::Indexed(177),
                        syn_string: Color::Indexed(150),
                        syn_var: Color::Indexed(117),
                        syn_text: Color::Indexed(252),
                        badge_fg: Color::Black,
                        badge_bg: Color::Indexed(75),
                    }
                } else {
                    Self {
                        mode,
                        bg_status_top: Color::Rgb(22, 24, 33),
                        bg_status_bottom: Color::Rgb(19, 21, 29),
                        bg_exec_line: Color::Rgb(30, 56, 40),
                        bg_history_line: Color::Rgb(62, 46, 22),
                        bg_cursor_line: Color::Rgb(30, 33, 46),
                        bg_watch_row: Color::Rgb(34, 30, 22),
                        bg_modal: Color::Rgb(20, 22, 30),
                        border_active: Color::Rgb(110, 175, 255),
                        border_inactive: Color::Rgb(54, 59, 74),
                        syn_comment: Color::Rgb(105, 112, 132),
                        syn_keyword: Color::Rgb(198, 145, 255),
                        syn_string: Color::Rgb(158, 206, 106),
                        syn_var: Color::Rgb(125, 207, 255),
                        syn_text: Color::Rgb(208, 214, 228),
                        badge_fg: Color::Black,
                        badge_bg: Color::Rgb(110, 175, 255),
                    }
                }
            }
        }
    }

    pub fn border(&self, focused: bool) -> Style {
        if focused {
            Style::default().fg(self.border_active)
        } else {
            Style::default().fg(self.border_inactive)
        }
    }

    pub fn title(&self, focused: bool) -> Style {
        if focused {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.syn_comment)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_theme_modes() {
        let m = ThemeMode::Classic;
        assert_eq!(m.next(), ThemeMode::LunaBlue);
        assert_eq!(m.next().next(), ThemeMode::TerminalNative);
        assert_eq!(m.next().next().next(), ThemeMode::Classic);
    }
}
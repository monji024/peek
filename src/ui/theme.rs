// https://github.com/monji024

use ratatui::{
    style::{Color, Modifier, Style},
    widgets::BorderType,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    LunaBlue,
    Classic,
    TerminalNative,
}

impl ThemeMode {
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "luna" | "luna-blue" | "lunablue" | "xp" => ThemeMode::LunaBlue,
            "native" | "terminal" | "terminal-native" | "ansi" => ThemeMode::TerminalNative,
            "classic" | "dark" => ThemeMode::Classic,
            _ => ThemeMode::LunaBlue,
        }
    }

    pub fn next(self) -> Self {
        match self {
            ThemeMode::LunaBlue => ThemeMode::Classic,
            ThemeMode::Classic => ThemeMode::TerminalNative,
            ThemeMode::TerminalNative => ThemeMode::LunaBlue,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::LunaBlue => "Luna Blue XP",
            ThemeMode::Classic => "Classic",
            ThemeMode::TerminalNative => "Terminal Native",
        }
    }

    pub fn border_type(self) -> BorderType {
        match self {
            ThemeMode::LunaBlue => BorderType::Double,
            ThemeMode::Classic => BorderType::Rounded,
            ThemeMode::TerminalNative => BorderType::Plain,
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
    pub bg_canvas: Color,
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
    pub accent_close_bg: Color,
}

impl Palette {
    pub fn for_mode(mode: ThemeMode, depth: ColorDepth) -> Self {
        if mode == ThemeMode::TerminalNative || depth == ColorDepth::Ansi16 {
            return Self {
                mode,
                bg_canvas: Color::Reset,
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
                badge_bg: Color::Green,
                accent_close_bg: Color::Red,
            };
        }

        match mode {
            ThemeMode::LunaBlue => {
                if depth == ColorDepth::Ansi256 {
                    Self {
                        mode,
                        bg_canvas: Color::Indexed(17),
                        bg_status_top: Color::Indexed(27),
                        bg_status_bottom: Color::Indexed(25),
                        bg_exec_line: Color::Indexed(22),
                        bg_history_line: Color::Indexed(94),
                        bg_cursor_line: Color::Indexed(24),
                        bg_watch_row: Color::Indexed(18),
                        bg_modal: Color::Indexed(17),
                        border_active: Color::Indexed(39),
                        border_inactive: Color::Indexed(61),
                        syn_comment: Color::Indexed(110),
                        syn_keyword: Color::Indexed(220),
                        syn_string: Color::Indexed(119),
                        syn_var: Color::Indexed(117),
                        syn_text: Color::Indexed(255),
                        badge_fg: Color::White,
                        badge_bg: Color::Indexed(34),
                        accent_close_bg: Color::Indexed(160),
                    }
                } else {
                    Self {
                        mode,
                        bg_canvas: Color::Rgb(12, 26, 62),
                        bg_status_top: Color::Rgb(0, 84, 227),
                        bg_status_bottom: Color::Rgb(36, 93, 219),
                        bg_exec_line: Color::Rgb(28, 88, 48),
                        bg_history_line: Color::Rgb(104, 72, 16),
                        bg_cursor_line: Color::Rgb(24, 58, 128),
                        bg_watch_row: Color::Rgb(18, 44, 98),
                        bg_modal: Color::Rgb(14, 32, 74),
                        border_active: Color::Rgb(84, 168, 255),
                        border_inactive: Color::Rgb(48, 86, 160),
                        syn_comment: Color::Rgb(142, 172, 224),
                        syn_keyword: Color::Rgb(255, 210, 76),
                        syn_string: Color::Rgb(134, 236, 126),
                        syn_var: Color::Rgb(122, 218, 255),
                        syn_text: Color::Rgb(242, 246, 255),
                        badge_fg: Color::White,
                        badge_bg: Color::Rgb(60, 168, 46),
                        accent_close_bg: Color::Rgb(222, 72, 43),
                    }
                }
            }
            ThemeMode::Classic | ThemeMode::TerminalNative => {
                if depth == ColorDepth::Ansi256 {
                    Self {
                        mode,
                        bg_canvas: Color::Reset,
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
                        accent_close_bg: Color::Indexed(160),
                    }
                } else {
                    Self {
                        mode,
                        bg_canvas: Color::Reset,
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
                        accent_close_bg: Color::Rgb(210, 68, 68),
                    }
                }
            }
        }
    }

    pub fn border(&self, focused: bool) -> Style {
        if focused {
            Style::default()
                .fg(self.border_active)
                .bg(self.bg_canvas)
        } else {
            Style::default()
                .fg(self.border_inactive)
                .bg(self.bg_canvas)
        }
    }

    pub fn title(&self, focused: bool) -> Style {
        if focused {
            Style::default()
                .fg(Color::White)
                .bg(if self.mode == ThemeMode::LunaBlue {
                    self.bg_status_top
                } else {
                    self.bg_canvas
                })
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(self.syn_comment)
                .bg(self.bg_canvas)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_theme_modes() {
        let m = ThemeMode::LunaBlue;
        assert_eq!(m.next(), ThemeMode::Classic);
        assert_eq!(m.next().next(), ThemeMode::TerminalNative);
        assert_eq!(m.next().next().next(), ThemeMode::LunaBlue);
    }
}
// https://github.com/monji024

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn normalize_command_key(event: KeyEvent) -> KeyCode {
    match event.code {
        KeyCode::Char(ch) => {
            let mapped = map_non_latin_char(ch);
            if event.modifiers.contains(KeyModifiers::SHIFT) {
                KeyCode::Char(mapped.to_ascii_uppercase())
            } else {
                KeyCode::Char(mapped)
            }
        }
        other => other,
    }
}
// سلاطین خواستید روی پروژ کار کنید میتونید زبان های دیگه رو هم به همین ترتیب اد بزنید 
// Folks if you're hacking on this, feel free to add other keyboard layouts the same way
fn map_non_latin_char(ch: char) -> char {
    match ch {
        'ض' => 'q',
        'ص' => 'w',
        'ث' => 'e',
        'ق' => 'r',
        'ف' => 't',
        'غ' => 'y',
        'ع' => 'u',
        'ه' => 'i',
        'خ' => 'o',
        'ح' => 'p',
        'ج' => '[',
        'چ' => ']',
        'ش' => 'a',
        'س' => 's',
        'ی' | 'ي' => 'd',
        'ب' => 'f',
        'ل' => 'g',
        'ا' | 'آ' => 'h',
        'ت' => 'j',
        'ن' => 'k',
        'م' => 'l',
        'ک' | 'ك' => ';',
        'گ' => '\'',
        'ظ' => 'z',
        'ط' => 'x',
        'ز' => 'c',
        'ر' => 'v',
        'ذ' => 'b',
        'د' => 'n',
        'پ' => 'm',
        'و' => ',',
        '؟' => '?',
        '÷' => '/',
        'ئ' | 'ء' => 'S',
        'ژ' => 'C',
        'إ' => 'B',
        'أ' => 'H',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_persian_layout_keys() {
        assert_eq!(map_non_latin_char('س'), 's');
        assert_eq!(map_non_latin_char('د'), 'n');
        assert_eq!(map_non_latin_char('ز'), 'c');
        assert_eq!(map_non_latin_char('ض'), 'q');
        assert_eq!(map_non_latin_char('ه'), 'i');
        assert_eq!(map_non_latin_char('ب'), 'f');
    }
}
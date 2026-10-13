//! Helvetica metrics (Adobe AFM, standard 14 fonts) and WinAnsi encoding, so free-text boxes
//! can be sized exactly like the text a viewer draws with the non-embedded Helvetica font.

/// Advance widths in 1/1000 em for WinAnsi codes 32..=255 (Helvetica.afm).
#[rustfmt::skip]
pub(crate) const WIDTHS: [u16; 224] = [
    // 32..=63:  space ! " # $ % & ' ( ) * + , - . / 0-9 : ; < = > ?
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
    // 64..=95:  @ A-Z [ \ ] ^ _
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
    // 96..=127: ` a-z { | } ~ (127 unused)
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, 556,
    // 128..=159: € . ‚ ƒ „ … † ‡ ˆ ‰ Š ‹ Œ . Ž .  . ‘ ’ “ ” • – — ˜ ™ š › œ . ž Ÿ
    556, 556, 222, 556, 333, 1000, 556, 556, 333, 1000, 667, 333, 1000, 556, 611, 556,
    556, 222, 222, 333, 333, 350, 556, 1000, 333, 1000, 500, 333, 944, 556, 500, 667,
    // 160..=191: nbsp ¡ ¢ £ ¤ ¥ ¦ § ¨ © ª « ¬ shy ® ¯ ° ± ² ³ ´ µ ¶ · ¸ ¹ º » ¼ ½ ¾ ¿
    278, 333, 556, 556, 556, 556, 260, 556, 333, 737, 370, 556, 584, 333, 737, 333,
    400, 584, 333, 333, 333, 556, 537, 278, 333, 333, 365, 556, 834, 834, 834, 611,
    // 192..=223: À-Å Æ Ç È-Ë Ì-Ï Ð Ñ Ò-Ö × Ø Ù-Ü Ý Þ ß
    667, 667, 667, 667, 667, 667, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278,
    722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611,
    // 224..=255: à-å æ ç è-ë ì-ï ð ñ ò-ö ÷ ø ù-ü ý þ ÿ
    556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500,
];

/// WinAnsi byte for `c`; characters outside the encoding become `?`.
pub fn win_ansi(c: char) -> u8 {
    let code = u32::from(c);
    match code {
        0x20..=0x7E | 0xA0..=0xFF => code as u8,
        _ => match c {
            '€' => 0x80,
            '‚' => 0x82,
            'ƒ' => 0x83,
            '„' => 0x84,
            '…' => 0x85,
            '†' => 0x86,
            '‡' => 0x87,
            'ˆ' => 0x88,
            '‰' => 0x89,
            'Š' => 0x8A,
            '‹' => 0x8B,
            'Œ' => 0x8C,
            'Ž' => 0x8E,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '˜' => 0x98,
            '™' => 0x99,
            'š' => 0x9A,
            '›' => 0x9B,
            'œ' => 0x9C,
            'ž' => 0x9E,
            'Ÿ' => 0x9F,
            _ => b'?',
        },
    }
}

/// Width of one WinAnsi code in 1/1000 em.
fn code_width(code: u8) -> u16 {
    code.checked_sub(32)
        .and_then(|index| WIDTHS.get(usize::from(index)))
        .copied()
        .unwrap_or(556)
}

/// Width of a single line of `text` in points at `font_size`.
pub fn line_width(text: &str, font_size: f32) -> f32 {
    let units: u32 = text
        .chars()
        .map(|c| u32::from(code_width(win_ansi(c))))
        .sum();
    units as f32 / 1000.0 * font_size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_is_2278_units() {
        assert!((line_width("Hello", 1000.0) - 2278.0).abs() < 1e-3);
    }

    #[test]
    fn cp1252_quotes_and_latin1_are_encoded() {
        assert_eq!(win_ansi('’'), 0x92);
        assert_eq!(win_ansi('é'), 0xE9);
        assert_eq!(win_ansi('漢'), b'?');
        assert!((line_width("é", 1000.0) - 556.0).abs() < 1e-3);
    }
}

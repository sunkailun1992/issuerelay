//! Cleaning of text that comes from the AI, Linear or the operating system before it is stored
//! or shown by the panel, the CLI and MCP.

/// The most characters of a summary or a reason that are kept.
const MAX_TEXT_CHARS: usize = 500;

/// Text from the AI, Linear or the operating system as one plain line, safe to show in the panel,
/// the CLI and MCP: every control character (newline and tab included) becomes a space, zero-width
/// and direction-changing characters are removed, the ends are trimmed and at most
/// `MAX_TEXT_CHARS` characters are kept, with `…` added when the text was cut.
pub fn clean_text(text: &str) -> String {
    let flat: String = text
        .chars()
        .filter_map(|c| {
            if c.is_control() {
                Some(' ')
            } else if is_invisible(c) {
                None
            } else {
                Some(c)
            }
        })
        .collect();
    let flat = flat.trim();
    match flat.char_indices().nth(MAX_TEXT_CHARS) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat.to_string(),
    }
}

/// Zero-width, line-separating and direction-changing characters. `char::is_control` lets them
/// through, but they make text show differently from what it says.
pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_are_single_line_and_capped() {
        assert_eq!(
            clean_text(" pushed\nto\ttest\u{1b}[2J "),
            "pushed to test [2J"
        );
        let long = clean_text(&"x".repeat(2000));
        assert_eq!(long.chars().count(), 501);
        assert!(long.ends_with('…'));
        assert_eq!(clean_text("短摘要"), "短摘要");
    }

    #[test]
    fn invisible_and_bidirectional_characters_are_removed() {
        // They survive `char::is_control`, yet make text read differently from what it says.
        for c in [
            '\u{200b}', '\u{200c}', '\u{200d}', '\u{200e}', '\u{200f}', '\u{2028}', '\u{2029}',
            '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}', '\u{2060}', '\u{2064}',
            '\u{2066}', '\u{2069}', '\u{206f}', '\u{feff}',
        ] {
            assert_eq!(clean_text(&format!("a{c}b")), "ab", "U+{:04X}", c as u32);
        }
        // Their neighbours, other punctuation and other scripts stay.
        for c in [
            '\u{200a}', '\u{2010}', '\u{2026}', '\u{2027}', '\u{202f}', '\u{205f}', '\u{2070}',
            '\u{fefe}', '\u{ff00}', 'é', '日',
        ] {
            let text = format!("a{c}b");
            assert_eq!(clean_text(&text), text, "U+{:04X}", c as u32);
        }
        // Removed characters do not count towards the length limit.
        let spread: String = (0..600).map(|_| "x\u{200b}").collect();
        let cleaned = clean_text(&spread);
        assert_eq!(cleaned.chars().count(), 501);
        assert!(cleaned.starts_with("xxx") && cleaned.ends_with('…'));
        // Nothing is left of a text that is all invisible.
        assert_eq!(clean_text("\u{202e}\u{200b}\n"), "");
    }
}

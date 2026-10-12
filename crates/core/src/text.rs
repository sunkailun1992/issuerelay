//! Cleaning of text that comes from the AI, Linear or the operating system before it is stored
//! or shown by the panel, the CLI and MCP.

/// The most characters of a summary or a reason that are kept.
const MAX_TEXT_CHARS: usize = 500;

/// Text from the AI, Linear or the operating system as one plain line, safe to show in the panel,
/// the CLI and MCP: every control character (newline and tab included) becomes a space, invisible
/// characters (`is_invisible`: zero-width, direction-changing and the like) are removed, the ends
/// are trimmed and at most `MAX_TEXT_CHARS` characters are kept, with `…` added when the text was
/// cut.
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

/// Characters that show nothing yet change how text reads: zero-width spaces, direction marks,
/// line and paragraph separators, bidi overrides and isolates, invisible operators, the byte
/// order mark, the soft hyphen, the Arabic letter mark, Hangul fillers and the tag characters.
/// `char::is_control` lets them through. The zero-width joiner and non-joiner (U+200C, U+200D)
/// are kept: emoji sequences and Indic scripts need them, and they cannot reorder text.
pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{200B}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{E0000}'..='\u{E007F}'
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
        // They survive `char::is_control`, yet make text read differently from what it says: the
        // soft hyphen, the Arabic letter mark, the Hangul fillers (blank letters), the tag
        // characters (a hidden channel for text) and the zero-width, direction and operator marks.
        let hidden = [
            '\u{00ad}', '\u{061c}', '\u{115f}', '\u{1160}', '\u{200b}', '\u{200e}', '\u{200f}',
            '\u{2028}', '\u{2029}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}',
            '\u{2060}', '\u{2064}', '\u{2066}', '\u{2069}', '\u{206f}', '\u{3164}', '\u{feff}',
            '\u{ffa0}',
        ];
        let tags = ['\u{e0001}', '\u{e0041}', '\u{e007f}'];
        for c in hidden.into_iter().chain(tags) {
            assert_eq!(clean_text(&format!("a{c}b")), "ab", "U+{:04X}", c as u32);
        }
        // Their neighbours, other punctuation and other scripts stay: the Hangul letters next to
        // the fillers, the variation selectors that make an emoji, the code points just outside
        // the tag block.
        let shown = [
            '\u{00ac}', '\u{00ae}', '\u{061b}', '\u{061d}', '\u{115e}', '\u{1161}', '\u{200a}',
            '\u{2010}', '\u{2026}', '\u{2027}', '\u{202f}', '\u{205f}', '\u{2070}', '\u{3163}',
            '\u{3165}', '\u{fe0f}', '\u{fefe}', '\u{ff00}', '\u{ff9f}', '\u{ffa1}', 'é', '日',
            '한',
        ];
        let outside_the_tags = ['\u{dffff}', '\u{e0080}', '\u{e0100}'];
        for c in shown.into_iter().chain(outside_the_tags) {
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

    #[test]
    fn joiners_stay_so_emoji_and_indic_text_keep_their_shape() {
        let family = "👨\u{200d}👩\u{200d}👧";
        assert_eq!(clean_text(family), family);
        let hindi = "क्\u{200d}ष";
        assert_eq!(clean_text(hindi), hindi);
        assert!(!is_invisible('\u{200c}') && !is_invisible('\u{200d}'));
        // The non-joiner keeps its place too, in Persian for one.
        let persian = "می\u{200c}خواهم";
        assert_eq!(clean_text(persian), persian);
    }
}

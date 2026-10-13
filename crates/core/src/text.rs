//! Cleaning of text that comes from the AI, Linear or the operating system before it is stored
//! or shown by the panel, the CLI and MCP.

use std::borrow::Cow;

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
/// are kept: emoji sequences and Indic scripts need them, and they cannot reorder text. Removing
/// the tag characters has a price: the flags of England, Scotland and Wales are a black flag
/// followed by tag characters, so they become a plain black flag. That is accepted, because tag
/// characters can also carry hidden text.
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

/// `text` without ANSI CSI sequences: `ESC [`, parameter bytes (`0`-`?`), intermediate bytes
/// (space-`/`) and one final byte (`@`-`~`). A CLI may colour what it prints into a pipe when
/// colour is forced (`FORCE_COLOR`), which would hide the words and URLs the parsers look for.
/// Anything that is not a complete CSI sequence stays as it is.
pub fn strip_ansi(text: &str) -> Cow<'_, str> {
    if !text.contains('\u{1b}') {
        return Cow::Borrowed(text);
    }
    let mut stripped = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("\u{1b}[") {
        stripped.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let bytes = after.as_bytes();
        let mut end = 0;
        while end < bytes.len() && matches!(bytes[end], 0x30..=0x3f) {
            end += 1;
        }
        while end < bytes.len() && matches!(bytes[end], 0x20..=0x2f) {
            end += 1;
        }
        if end < bytes.len() && matches!(bytes[end], 0x40..=0x7e) {
            // Every byte up to and including the final one is ASCII, so this is a char boundary.
            rest = &after[end + 1..];
        } else {
            stripped.push_str("\u{1b}[");
            rest = after;
        }
    }
    stripped.push_str(rest);
    Cow::Owned(stripped)
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

    #[test]
    fn the_flags_of_england_scotland_and_wales_lose_their_tag_characters() {
        // Such a flag is a black flag followed by tag characters that spell the region and end
        // with U+E007F. They are removed like any tag character, which can carry hidden text, so
        // the plain black flag is what is left. That loss is accepted.
        let england = "\u{1f3f4}\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}";
        assert_eq!(clean_text(england), "\u{1f3f4}");
        assert_eq!(clean_text(&format!("a{england}b")), "a\u{1f3f4}b");
    }

    #[test]
    fn ansi_csi_sequences_are_stripped_and_nothing_else() {
        // Text without an escape is returned as it is, without a copy.
        assert!(matches!(
            strip_ansi("plain ✔ text"),
            Cow::Borrowed("plain ✔ text")
        ));
        assert_eq!(strip_ansi("\u{1b}[36mhttps://x\u{1b}[39m"), "https://x");
        // Several parameters, no parameter, other final bytes, a private marker, an intermediate
        // byte, and multi-byte text right next to a sequence.
        assert_eq!(strip_ansi("\u{1b}[1;31mred\u{1b}[0m"), "red");
        assert_eq!(strip_ansi("a\u{1b}[mb"), "ab");
        assert_eq!(strip_ansi("a\u{1b}[2Kb\u{1b}[?25lc\u{1b}[~d"), "abcd");
        assert_eq!(strip_ansi("a\u{1b}[2 qb"), "ab");
        // The final byte may be any of `@` to `~`, and nothing outside that range.
        assert_eq!(strip_ansi("a\u{1b}[3@b"), "ab");
        assert_eq!(strip_ansi("a\u{1b}[3\u{7f}b"), "a\u{1b}[3\u{7f}b");
        assert_eq!(strip_ansi("\u{1b}[32m✔ Connected\u{1b}[39m"), "✔ Connected");
        // Not a complete CSI sequence: left alone, and it does not hide a good one after it.
        assert_eq!(strip_ansi("a\u{1b}[32"), "a\u{1b}[32");
        assert_eq!(strip_ansi("a\u{1b}[ ✔"), "a\u{1b}[ ✔");
        assert_eq!(strip_ansi("a\u{1b}[\u{7}b"), "a\u{1b}[\u{7}b");
        assert_eq!(strip_ansi("\u{1b}[ \u{1b}[32mok\u{1b}[0m"), "\u{1b}[ ok");
        // Escapes that are not CSI are not touched here.
        assert_eq!(strip_ansi("a\u{1b}]8;;x\u{7}b"), "a\u{1b}]8;;x\u{7}b");
        assert_eq!(strip_ansi("a\u{1b}Mb"), "a\u{1b}Mb");
    }
}

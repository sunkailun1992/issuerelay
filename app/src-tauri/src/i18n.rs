//! The interface languages, read from the panel's translation files so that the tray menu,
//! macOS's application menu and the notifications say the same as the panel. (The quit prompt and
//! the startup error page are the panel's own text.)
use std::sync::LazyLock;

use issuerelay_core::config::Settings;
use serde_json::Value;

/// The interface languages; the panel offers the same.
pub const LANGUAGES: [&str; 5] = ["zh-CN", "zh-TW", "en", "ja", "ko"];

/// The panel's translation files, embedded at build time.
const SOURCES: [(&str, &str); 5] = [
    ("zh-CN", include_str!("../../src/i18n/locales/zh-CN.json")),
    ("zh-TW", include_str!("../../src/i18n/locales/zh-TW.json")),
    ("en", include_str!("../../src/i18n/locales/en.json")),
    ("ja", include_str!("../../src/i18n/locales/ja.json")),
    ("ko", include_str!("../../src/i18n/locales/ko.json")),
];

/// Every key the Rust side looks up; the tests check that each language has them.
pub const USED_KEYS: &[&str] = &[
    "tray.starting",
    "tray.status_counts",
    "tray.status_paused",
    "tray.status_problem",
    "tray.pause",
    "tray.resume",
    "tray.poll_now",
    "tray.open_panel",
    "tray.quit",
    "menu.about",
    "menu.services",
    "menu.hide",
    "menu.hide_others",
    "menu.file",
    "menu.close_window",
    "menu.edit",
    "menu.undo",
    "menu.redo",
    "menu.cut",
    "menu.copy",
    "menu.paste",
    "menu.select_all",
    "menu.view",
    "menu.fullscreen",
    "menu.window",
    "menu.minimize",
    "menu.zoom",
    "menu.help",
    "notify.run_started",
    "notify.awaiting_acceptance",
    "notify.needs_info",
    "notify.failed",
];

/// The interface language for a `language` setting ("system" or one of `LANGUAGES`) and the
/// system locale. For Chinese the script subtag decides first (Hans, Hant), then the region.
/// Keep in step with `resolveLanguage` in app/src/i18n/language.ts: both are tested with
/// app/src/i18n/language-cases.json.
pub fn resolve_language(setting: &str, system: Option<&str>) -> &'static str {
    if let Some(lang) = LANGUAGES.iter().find(|lang| **lang == setting) {
        return lang;
    }
    if setting != "system" {
        return "en";
    }
    let locale = system.unwrap_or_default().to_ascii_lowercase();
    let mut subtags = locale.split(['-', '_']);
    match subtags.next() {
        Some("zh") => {
            let rest: Vec<&str> = subtags.collect();
            // The script wins over the region: zh-Hans-HK is Simplified Chinese.
            let traditional = !rest.contains(&"hans")
                && (rest.contains(&"hant")
                    || rest.iter().any(|tag| matches!(*tag, "tw" | "hk" | "mo")));
            if traditional { "zh-TW" } else { "zh-CN" }
        }
        Some("ja") => "ja",
        Some("ko") => "ko",
        _ => "en",
    }
}

/// Every language's table, parsed once, the first time any text is looked up.
static TABLES: LazyLock<Vec<(&'static str, Value)>> = LazyLock::new(|| {
    SOURCES
        .iter()
        .map(|(lang, text)| {
            let table = serde_json::from_str(text)
                .expect("translation files are valid JSON (checked by the panel's tests)");
            (*lang, table)
        })
        .collect()
});

/// The table of `lang`; English for a language IssueRelay does not offer.
fn table(lang: &str) -> &'static Value {
    let find = |wanted: &str| {
        TABLES
            .iter()
            .find(|(name, _)| *name == wanted)
            .map(|(_, table)| table)
    };
    find(lang)
        .or_else(|| find("en"))
        .expect("the English translations are embedded")
}

/// The text at the dotted `key` (`tray.quit`); `None` when the key is missing or is not a text.
fn lookup<'a>(table: &'a Value, key: &str) -> Option<&'a str> {
    key.split('.')
        .try_fold(table, |node, part| node.get(part))?
        .as_str()
}

/// Looks up interface texts in one language, falling back to English, then to the key itself.
/// Cheap to make: it only points at tables parsed once (`TABLES`).
pub struct Translator<'a> {
    table: &'a Value,
    fallback: &'a Value,
}

impl Translator<'static> {
    /// Texts in `lang`; English when IssueRelay does not offer that language.
    pub fn new(lang: &str) -> Self {
        Self {
            table: table(lang),
            fallback: table("en"),
        }
    }

    /// The language the settings ask for (`"system"` follows the system locale).
    pub fn for_settings(settings: &Settings) -> Self {
        Self::new(resolve_language(
            &settings.language,
            sys_locale::get_locale().as_deref(),
        ))
    }
}

impl Translator<'_> {
    /// The text for the dotted `key` (`tray.quit`): this language's, else English's, else the
    /// key itself, so a missing text shows up as the key and never as an empty menu entry.
    pub fn t(&self, key: &str) -> String {
        lookup(self.table, key)
            .or_else(|| lookup(self.fallback, key))
            .unwrap_or(key)
            .to_string()
    }

    /// `t` with each `{{name}}` replaced by its value.
    pub fn t_with(&self, key: &str, vars: &[(&str, &str)]) -> String {
        vars.iter().fold(self.t(key), |text, (name, value)| {
            text.replace(&format!("{{{{{name}}}}}"), value)
        })
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    /// One row of app/src/i18n/language-cases.json, which the panel's tests read too.
    #[derive(Deserialize)]
    struct Case {
        setting: String,
        system: Option<String>,
        expected: String,
    }

    #[test]
    fn languages_resolve_like_the_panel_does() {
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../../src/i18n/language-cases.json"))
                .expect("language-cases.json is valid");
        assert!(cases.len() >= 19, "the shared cases were read");
        for case in cases {
            assert_eq!(
                resolve_language(&case.setting, case.system.as_deref()),
                case.expected,
                "{} / {:?}",
                case.setting,
                case.system
            );
        }
    }

    #[test]
    fn every_language_is_listed_alike_and_embeds_its_own_file() {
        // The panel's list (app/src/i18n/language.ts), in the same order: a language added on one
        // side only would leave the tray in another language than the panel.
        let panel = include_str!("../../src/i18n/language.ts");
        let line = panel
            .lines()
            .find(|line| line.starts_with("export const LANGUAGES"))
            .expect("language.ts lists the languages");
        let listed: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
        assert_eq!(listed, LANGUAGES, "language.ts and LANGUAGES");
        let embedded: Vec<&str> = SOURCES.iter().map(|(lang, _)| *lang).collect();
        assert_eq!(embedded, LANGUAGES, "SOURCES and LANGUAGES");
        // Each language embeds its own file, not another language's.
        for (lang, text) in SOURCES {
            let file = format!(
                "{}/../src/i18n/locales/{lang}.json",
                env!("CARGO_MANIFEST_DIR")
            );
            assert!(
                text == std::fs::read_to_string(&file).unwrap(),
                "{lang} does not embed {file}"
            );
        }
    }

    #[test]
    fn every_key_rust_uses_exists_in_every_language() {
        for lang in LANGUAGES {
            for key in USED_KEYS {
                assert!(lookup(table(lang), key).is_some(), "{lang} lacks {key}");
            }
        }
    }

    #[test]
    fn missing_keys_fall_back_to_english_then_to_the_key() {
        // Small tables of its own, so the English fallback is really taken: the real files have
        // the same keys in every language (the panel's tests check that).
        let en = json!({ "tray": { "quit": "Quit IssueRelay", "pause": "Pause monitoring" } });
        let ja = json!({ "tray": { "quit": "IssueRelay を終了" } });
        let t = Translator {
            table: &ja,
            fallback: &en,
        };
        assert_eq!(t.t("tray.quit"), "IssueRelay を終了");
        assert_eq!(
            t.t("tray.pause"),
            "Pause monitoring",
            "missing in Japanese: English"
        );
        assert_eq!(
            t.t("no.such.key"),
            "no.such.key",
            "missing everywhere: the key itself"
        );
    }

    #[test]
    fn placeholders_are_filled_in() {
        assert_eq!(
            Translator::new("en")
                .t_with("tray.status_counts", &[("running", "2"), ("queued", "1")]),
            "Running 2 · Queued 1"
        );
        assert_eq!(
            Translator::new("xx").t("tray.quit"),
            "Quit IssueRelay",
            "an unknown language reads English"
        );
    }
}

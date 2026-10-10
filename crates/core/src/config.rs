//! User settings and Linear-project bindings, stored as TOML.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutorKind {
    Claude,
    Codex,
}

impl ExecutorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutorKind::Claude => "claude",
            ExecutorKind::Codex => "codex",
        }
    }

    pub fn program_name(self) -> &'static str {
        self.as_str()
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "claude" => Some(ExecutorKind::Claude),
            "codex" => Some(ExecutorKind::Codex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionProfile {
    /// Skip every permission check (spec default).
    Full,
    /// Let the executor's own automatic reviewer decide.
    Auto,
    /// Use only what the user already allowed in their own settings.
    Personal,
}

impl PermissionProfile {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "full" => Some(PermissionProfile::Full),
            "auto" => Some(PermissionProfile::Auto),
            "personal" => Some(PermissionProfile::Personal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TriggerNames {
    pub label: String,
    pub todo: String,
    pub in_progress: String,
    pub in_testing: String,
    pub done: String,
}

impl Default for TriggerNames {
    fn default() -> Self {
        Self {
            label: "ai-ready".into(),
            todo: "Todo".into(),
            in_progress: "In Progress".into(),
            in_testing: "In Testing".into(),
            done: "Done".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "system" or a BCP 47 tag such as "zh-CN".
    pub language: String,
    pub poll_interval_secs: u64,
    pub max_concurrent: usize,
    pub run_timeout_mins: u64,
    pub stall_timeout_mins: u64,
    pub default_executor: ExecutorKind,
    pub default_permission: PermissionProfile,
    pub prompt_template: String,
    /// "follow-ui" or a language tag for comments the AI writes in Linear.
    pub comment_language: String,
    pub paused: bool,
    pub trigger: TriggerNames,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "system".into(),
            poll_interval_secs: 30,
            max_concurrent: 2,
            run_timeout_mins: 60,
            stall_timeout_mins: 15,
            default_executor: ExecutorKind::Claude,
            default_permission: PermissionProfile::Full,
            prompt_template: String::new(),
            comment_language: "follow-ui".into(),
            paused: false,
            trigger: TriggerNames::default(),
        }
    }
}

impl Settings {
    pub fn resolved_language(&self) -> String {
        if self.language == "system" {
            sys_locale::get_locale().unwrap_or_else(|| "en-US".into())
        } else {
            self.language.clone()
        }
    }

    pub fn resolved_comment_language(&self) -> String {
        if self.comment_language == "follow-ui" {
            self.resolved_language()
        } else {
            self.comment_language.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub directory: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<ExecutorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_override: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub settings: Settings,
    pub bindings: Vec<Binding>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("invalid config file {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    /// Writes to a temporary file first, then renames it over the old one.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("cannot replace {}", path.display()))?;
        Ok(())
    }

    pub fn binding_for_project(&self, project_id: &str) -> Option<&Binding> {
        self.bindings.iter().find(|b| b.project_id == project_id)
    }

    pub fn executor_for(&self, binding: &Binding) -> ExecutorKind {
        binding.executor.unwrap_or(self.settings.default_executor)
    }

    pub fn permission_for(&self, binding: &Binding) -> PermissionProfile {
        binding
            .permission
            .unwrap_or(self.settings.default_permission)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn binding(project: &str, executor: Option<ExecutorKind>) -> Binding {
        Binding {
            id: project.into(),
            project_id: project.into(),
            project_name: "Demo Project".into(),
            directory: PathBuf::from("demo"),
            executor,
            permission: None,
            model: None,
            prompt_override: None,
        }
    }

    #[test]
    fn defaults_match_spec() {
        let s = Settings::default();
        assert_eq!(s.language, "system");
        assert_eq!(s.poll_interval_secs, 30);
        assert_eq!(s.max_concurrent, 2);
        assert_eq!(s.run_timeout_mins, 60);
        assert_eq!(s.stall_timeout_mins, 15);
        assert_eq!(s.default_executor, ExecutorKind::Claude);
        assert_eq!(s.default_permission, PermissionProfile::Full);
        assert_eq!(s.comment_language, "follow-ui");
        assert!(!s.paused);
        assert_eq!(s.trigger.label, "ai-ready");
        assert_eq!(s.trigger.todo, "Todo");
        assert_eq!(s.trigger.in_progress, "In Progress");
        assert_eq!(s.trigger.in_testing, "In Testing");
        assert_eq!(s.trigger.done, "Done");
    }

    #[test]
    fn missing_file_loads_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Config::load(&dir.path().join("none.toml")).unwrap(),
            Config::default()
        );
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let mut cfg = Config::default();
        cfg.settings.max_concurrent = 3;
        cfg.bindings
            .push(binding("proj-1", Some(ExecutorKind::Codex)));
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), cfg);
        cfg.settings.paused = true;
        cfg.save(&path).unwrap();
        assert!(Config::load(&path).unwrap().settings.paused);
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[settings]\npoll_interval_secs = 10\n").unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.settings.poll_interval_secs, 10);
        assert_eq!(cfg.settings.max_concurrent, 2);
        assert!(cfg.bindings.is_empty());
    }

    #[test]
    fn invalid_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "settings = 5\n").unwrap();
        assert!(Config::load(&path).is_err());
    }

    #[test]
    fn binding_lookup_and_effective_choices() {
        let mut cfg = Config::default();
        cfg.bindings.push(binding("proj-1", None));
        cfg.bindings.push(Binding {
            permission: Some(PermissionProfile::Auto),
            ..binding("proj-2", Some(ExecutorKind::Codex))
        });
        let b1 = cfg.binding_for_project("proj-1").unwrap();
        assert_eq!(cfg.executor_for(b1), ExecutorKind::Claude);
        assert_eq!(cfg.permission_for(b1), PermissionProfile::Full);
        let b2 = cfg.binding_for_project("proj-2").unwrap();
        assert_eq!(cfg.executor_for(b2), ExecutorKind::Codex);
        assert_eq!(cfg.permission_for(b2), PermissionProfile::Auto);
        assert!(cfg.binding_for_project("proj-3").is_none());
    }

    #[test]
    fn comment_language_resolution() {
        let mut s = Settings {
            language: "ja".into(),
            ..Settings::default()
        };
        assert_eq!(s.resolved_comment_language(), "ja");
        s.comment_language = "en".into();
        assert_eq!(s.resolved_comment_language(), "en");
    }

    #[test]
    fn names_parse_case_insensitively() {
        assert_eq!(ExecutorKind::parse(" Codex "), Some(ExecutorKind::Codex));
        assert_eq!(ExecutorKind::parse("claude"), Some(ExecutorKind::Claude));
        assert_eq!(ExecutorKind::parse("gpt"), None);
        assert_eq!(
            PermissionProfile::parse("AUTO"),
            Some(PermissionProfile::Auto)
        );
        assert_eq!(
            PermissionProfile::parse("personal"),
            Some(PermissionProfile::Personal)
        );
    }
}

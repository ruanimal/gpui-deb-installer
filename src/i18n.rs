use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    Chinese,
}

static DETECTED_LANGUAGE: OnceLock<Language> = OnceLock::new();

pub fn current_language() -> Language {
    *DETECTED_LANGUAGE.get_or_init(detect_language)
}

pub fn tr<'a>(en: &'a str, zh: &'a str) -> &'a str {
    match current_language() {
        Language::Chinese => zh,
        Language::English => en,
    }
}

fn detect_language() -> Language {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANGUAGE", "LANG"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok())
        .unwrap_or_default();

    let normalized = locale
        .split(':')
        .next()
        .unwrap_or("")
        .split('.')
        .next()
        .unwrap_or("")
        .split('@')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    if normalized.starts_with("zh") {
        Language::Chinese
    } else {
        Language::English
    }
}

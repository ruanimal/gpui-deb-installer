use std::sync::OnceLock;

static DETECTED_LOCALE: OnceLock<String> = OnceLock::new();

pub fn init_locale() {
    rust_i18n::set_locale(detect_locale());
}

pub fn tr(key: &str) -> String {
    crate::_rust_i18n_try_translate(&rust_i18n::locale(), key)
        .map(|s| s.into_owned())
        .unwrap_or_else(|| key.to_string())
}

pub fn locale() -> &'static str {
    DETECTED_LOCALE.get_or_init(detect_locale_inner).as_str()
}

fn detect_locale() -> &'static str {
    locale()
}

fn detect_locale_inner() -> String {
    let available = rust_i18n::available_locales!();

    for candidate in locale_candidates() {
        if let Some(matched) = match_available_locale(&candidate, &available) {
            return matched.to_string();
        }
    }

    if let Some(en) = available
        .iter()
        .copied()
        .find(|loc| normalize_locale(loc) == "en")
    {
        return en.to_string();
    }

    available
        .first()
        .copied()
        .unwrap_or("en")
        .to_string()
}

fn locale_candidates() -> Vec<String> {
    let mut result = Vec::new();
    for key in ["LC_ALL", "LC_MESSAGES", "LANGUAGE", "LANG"] {
        if let Ok(v) = std::env::var(key) {
            if key == "LANGUAGE" {
                result.extend(v.split(':').map(|s| s.to_string()));
            } else {
                result.push(v);
            }
        }
    }
    result
}

fn match_available_locale<'a>(candidate: &str, available: &'a [&'a str]) -> Option<&'a str> {
    let norm = normalize_locale(candidate);
    if norm.is_empty() {
        return None;
    }

    if let Some(exact) = available
        .iter()
        .copied()
        .find(|loc| normalize_locale(loc) == norm)
    {
        return Some(exact);
    }

    let lang = norm.split('-').next().unwrap_or("");
    if lang.is_empty() {
        return None;
    }

    available
        .iter()
        .copied()
        .find(|loc| normalize_locale(loc).split('-').next().unwrap_or("") == lang)
}

fn normalize_locale(input: &str) -> String {
    input
        .split('.')
        .next()
        .unwrap_or("")
        .split('@')
        .next()
        .unwrap_or("")
        .replace('_', "-")
        .to_ascii_lowercase()
}

use unicode_script::Script;

use super::Fallback;

#[derive(Debug)]
pub struct PlatformFallback;

impl Fallback for PlatformFallback {
    fn common_fallback(&self) -> &'static [&'static str] {
        common_fallback()
    }

    fn forbidden_fallback(&self) -> &'static [&'static str] {
        forbidden_fallback()
    }

    fn script_fallback(
        &self,
        script: unicode_script::Script,
        locale: &str,
    ) -> &'static [&'static str] {
        script_fallback(script, locale)
    }
}

fn common_fallback() -> &'static [&'static str] {
    &[
        "Segoe UI",
        "Segoe UI Emoji",
        "Segoe UI Symbol",
        "Segoe UI Historic",
    ]
}

fn forbidden_fallback() -> &'static [&'static str] {
    &[]
}

fn han_unification(locale: &str) -> &'static [&'static str] {
    match locale {
        "ja" => &["Yu Gothic"],
        "ko" => &["Malgun Gothic"],
        "zh-HK" => &["MingLiU_HKSCS"],
        "zh-TW" => &["Microsoft JhengHei UI"],
        _ => &["Microsoft YaHei UI"],
    }
}

fn script_fallback(script: Script, locale: &str) -> &'static [&'static str] {
    match script {
        Script::Adlam => &["Ebrima"],
        Script::Bengali => &["Nirmala UI"],
        Script::Canadian_Aboriginal => &["Gadugi"],
        Script::Chakma => &["Nirmala UI"],
        Script::Cherokee => &["Gadugi"],
        Script::Devanagari => &["Nirmala UI"],
        Script::Ethiopic => &["Ebrima"],
        Script::Gujarati => &["Nirmala UI"],
        Script::Gurmukhi => &["Nirmala UI"],
        Script::Han => han_unification(locale),
        Script::Hangul => han_unification("ko"),
        Script::Hiragana => han_unification("ja"),
        Script::Javanese => &["Javanese Text"],
        Script::Kannada => &["Nirmala UI"],
        Script::Katakana => han_unification("ja"),
        Script::Khmer => &["Leelawadee UI"],
        Script::Lao => &["Leelawadee UI"],
        Script::Malayalam => &["Nirmala UI"],
        Script::Mongolian => &["Mongolian Baiti"],
        Script::Myanmar => &["Myanmar Text"],
        Script::Oriya => &["Nirmala UI"],
        Script::Sinhala => &["Nirmala UI"],
        Script::Tamil => &["Nirmala UI"],
        Script::Telugu => &["Nirmala UI"],
        Script::Thaana => &["MV Boli"],
        Script::Thai => &["Leelawadee UI"],
        Script::Tibetan => &["Microsoft Himalaya"],
        Script::Tifinagh => &["Ebrima"],
        Script::Vai => &["Ebrima"],
        Script::Yi => &["Microsoft Yi Baiti"],
        _ => &[],
    }
}

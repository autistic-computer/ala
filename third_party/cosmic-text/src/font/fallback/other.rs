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

const fn common_fallback() -> &'static [&'static str] {
    &[]
}

const fn forbidden_fallback() -> &'static [&'static str] {
    &[]
}

const fn script_fallback(_script: Script, _locale: &str) -> &'static [&'static str] {
    &[]
}

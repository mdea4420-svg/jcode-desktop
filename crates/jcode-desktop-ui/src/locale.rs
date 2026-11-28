//! Language selected once per desktop process.

use std::sync::OnceLock;

pub(crate) fn is_zh() -> bool {
    #[cfg(test)]
    if TEST_CHINESE.with(std::cell::Cell::get) {
        return true;
    }
    static CHINESE: OnceLock<bool> = OnceLock::new();
    *CHINESE.get_or_init(|| {
        if let Ok(language) = std::env::var("JCODE_DESKTOP_LANGUAGE") {
            return language.eq_ignore_ascii_case("zh-CN");
        }
        crate::config::is_zh_preview_executable()
    })
}

pub(crate) fn tr(english: &'static str, chinese: &'static str) -> &'static str {
    if is_zh() { chinese } else { english }
}

pub(crate) const fn super_key() -> &'static str {
    if cfg!(target_os = "windows") {
        "Win"
    } else {
        "Super"
    }
}

#[cfg(test)]
thread_local! {
    static TEST_CHINESE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn with_zh<R>(test: impl FnOnce() -> R) -> R {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_CHINESE.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_CHINESE.with(|value| value.replace(true)));
    test()
}

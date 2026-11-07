//! The machine's *regional* locale.
//!
//! Only used to pick which language the labels speak. Windows' regional format
//! and its display language are two different settings — this machine pairs an
//! English format with a Spanish UI — and from JS they cannot be told apart:
//! in WebView2 both `navigator.language` and `Intl` follow the display
//! language. `GetUserDefaultLocaleName` is the regional one, which is the one
//! the user picked when setting the screen up.

/// Locale tag such as `en-US`, or empty when the platform has nothing to say
/// (the UI then falls back to its own detection).
#[cfg(windows)]
pub fn user_locale() -> String {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    // LOCALE_NAME_MAX_LENGTH is 85, and the call returns the character count
    // *including* the terminator.
    let mut buf = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if len <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize - 1])
}

/// POSIX shells already carry it; no need for a macOS-specific call here.
#[cfg(not(windows))]
pub fn user_locale() -> String {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_a_locale_tag_when_it_answers() {
        let tag = user_locale();
        if tag.is_empty() {
            return; // platform with no answer: a valid case
        }
        // "en-US", "es-ES", "C", "en_US.UTF-8"… the language is the first 2-3
        // letters; what matters is that one can be read.
        let lang: String = tag
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        assert!(
            (2..=3).contains(&lang.len()),
            "'{tag}' does not start with a 2-3 letter language"
        );
    }
}

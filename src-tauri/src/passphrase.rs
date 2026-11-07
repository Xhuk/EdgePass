//! Diceware passphrases, born inside the widget.
//!
//! The word list is published on purpose. What stays secret is **which** words
//! were drawn, never **which** words exist: 7.411 words carry 12,86 bits each, so
//! six of them are worth about 77 bits. Hiding the list would add nothing — anyone
//! can read it out of the binary — and would cost the one thing the list gives
//! away for free: anyone can check that the draw is uniform.
//!
//! The value never crosses the local pipe. It is built here and handed to the
//! window that asked, the same way the app already shows a secret it just read.

use serde::{Deserialize, Serialize};

/// The published Diceware list, filtered to `[a-z]` so the "compatible" variant
/// can promise alphanumerics only. 365 entries with apostrophes, symbols or
/// digits were dropped. See `THIRD-PARTY-NOTICES.md`.
static WORDS: &str = include_str!("../data/diceware-en.txt");

pub const MIN_WORDS: usize = 3;
pub const MAX_WORDS: usize = 12;
/// Digits appended when the caller asks for them.
const DIGITS: usize = 2;
/// Characters that survive a shell, a URL and a spreadsheet cell.
const SYMBOLS: &[u8] = b"!@#$%&*+-=?^_~";
/// Per-phrase random block. 62 symbols ≈ 5.95 bits each, drawn fresh every
/// call: this is the part that actually adds entropy, unlike a stored seed.
const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
pub const MAX_RANDOM: usize = 12;
/// The only separators accepted. A slash or a quote would break more call sites
/// than it helps: the value is pasted into config files, URLs and forms.
const SEPARATORS: [&str; 5] = ["-", "_", ".", " ", ""];
/// Below this the Diceware page suggests starting over. Reported as advice,
/// never as a refusal: the caller decides what their system accepts.
const SHORT_WARNING: usize = 19;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    #[serde(default = "default_words")]
    pub words: usize,
    #[serde(default = "default_separator")]
    pub separator: String,
    /// Upper-case the first letter of every word. Deterministic, so it adds no
    /// entropy: it only makes the phrase easier to read.
    #[serde(default)]
    pub capitalize: bool,
    #[serde(default)]
    pub digits: bool,
    #[serde(default)]
    pub symbol: bool,
    /// Characters from [`ALNUM`] appended to every phrase, drawn fresh. This is
    /// real entropy; 0 keeps the old shape.
    #[serde(default)]
    pub random_chars: usize,
    /// Force `[A-Za-z0-9]` for clients that reject anything else.
    #[serde(default)]
    pub compatible: bool,
}

fn default_words() -> usize {
    6
}

fn default_separator() -> String {
    "-".to_string()
}

impl Default for Options {
    fn default() -> Self {
        Self {
            words: default_words(),
            separator: default_separator(),
            capitalize: false,
            digits: false,
            symbol: false,
            random_chars: 0,
            compatible: false,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Made {
    pub value: String,
    pub words: usize,
    pub vocabulary: usize,
    pub bits_per_word: f64,
    pub entropy_bits: f64,
    pub length: usize,
    pub compatible: bool,
    /// How many characters of the phrase came from the fresh random block.
    pub random_chars: usize,
    /// Set when the phrase is shorter than this app would recommend.
    pub advice: Option<String>,
}

/// The word list, parsed once per call. 7.411 short words: splitting is cheaper
/// than holding a second copy in memory for the life of the process.
fn vocabulary() -> Vec<&'static str> {
    WORDS
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect()
}

/// Builds one passphrase.
///
/// Every word comes from [`crate::rand::below`], which rejects out-of-range
/// draws instead of taking a remainder, so no word is ever more likely than
/// another. The result is returned to the window and is never written to a log,
/// a file or the pipe.
/// Builds one passphrase with no installation salt.
///
/// Test-only: the command path always goes through [`make_with`] with the salt
/// from `config.json`, so without this attribute the release build reports the
/// function as dead code.
#[cfg(test)]
pub fn make(opts: &Options) -> Result<Made, String> {
    make_with(opts, &[])
}

/// Builds one passphrase, mixing a per-installation **salt** into the draw.
///
/// The salt is not a secret and adds **no entropy**: it is stored in
/// `config.json` so two machines cloned from the same snapshot (same OS RNG
/// state at boot) still map the same draw to different words. The fresh random
/// block is the part that adds bits; the words keep their own.
pub fn make_with(opts: &Options, salt: &[u8]) -> Result<Made, String> {
    let list = vocabulary();
    let n = list.len();
    if n < 1000 {
        return Err(format!(
            "the word list did not load correctly: {n} entries"
        ));
    }
    if opts.words < MIN_WORDS || opts.words > MAX_WORDS {
        return Err(format!(
            "word count out of range: {} ({MIN_WORDS} to {MAX_WORDS} accepted)",
            opts.words
        ));
    }
    if opts.random_chars > MAX_RANDOM {
        return Err(format!(
            "random block out of range: {} (maximum {MAX_RANDOM})",
            opts.random_chars
        ));
    }

    // "Compatible" overrides the options that would break a strict client.
    let separator = if opts.compatible {
        ""
    } else {
        opts.separator.as_str()
    };
    let symbol = !opts.compatible && opts.symbol;
    if !SEPARATORS.contains(&separator) {
        return Err(format!(
            "separator not allowed: {separator:?}. Valid ones are {SEPARATORS:?}"
        ));
    }

    let mut picked: Vec<String> = Vec::with_capacity(opts.words);
    for i in 0..opts.words {
        let idx = shifted_draw(n as u32, salt, i)? as usize;
        let word = list[idx];
        if opts.capitalize {
            let mut chars = word.chars();
            let capitalized = match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            };
            picked.push(capitalized);
        } else {
            picked.push(word.to_string());
        }
    }

    let mut value = picked.join(separator);
    // The fresh block goes before the suffixes so digits/symbol stay at the end
    // (the shape `digits` has always had).
    for i in 0..opts.random_chars {
        let idx = shifted_draw(ALNUM.len() as u32, salt, 3000 + i)? as usize;
        value.push(char::from(ALNUM[idx]));
    }
    if opts.digits {
        for i in 0..DIGITS {
            let d = shifted_draw(10, salt, 1000 + i)? as u8;
            value.push(char::from(b'0' + d));
        }
    }
    if symbol {
        let i = shifted_draw(SYMBOLS.len() as u32, salt, 2000)? as usize;
        value.push(char::from(SYMBOLS[i]));
    }

    let bits_per_word = (n as f64).log2();
    let mut entropy_bits = (opts.words as f64) * bits_per_word;
    if opts.random_chars > 0 {
        entropy_bits += (opts.random_chars as f64) * (ALNUM.len() as f64).log2();
    }
    if opts.digits {
        entropy_bits += (DIGITS as f64) * 10f64.log2();
    }
    if symbol {
        entropy_bits += (SYMBOLS.len() as f64).log2();
    }

    let length = value.chars().count();
    let advice = if length < SHORT_WARNING {
        Some(format!(
            "That left {length} characters. Some systems require {SHORT_WARNING} or more: one extra word fixes it."
        ))
    } else {
        None
    };

    Ok(Made {
        value,
        words: opts.words,
        vocabulary: n,
        bits_per_word,
        entropy_bits,
        length,
        compatible: opts.compatible,
        random_chars: opts.random_chars,
        advice,
    })
}

/// A uniform draw in `0..n`, rotated by the salt for this position.
///
/// Shifting a uniform value by a constant modulo `n` is still uniform, so the
/// salt never biases the vocabulary. With no salt it is exactly
/// [`crate::rand::below`].
fn shifted_draw(n: u32, salt: &[u8], position: usize) -> Result<u32, String> {
    let draw = crate::rand::below(n)? as u64;
    if salt.is_empty() {
        return Ok(draw as u32);
    }
    Ok(((draw + shift(salt, position)) % n as u64) as u32)
}

/// FNV-1a over the salt and the position. Not a security boundary: the salt is
/// public. It only has to spread distinct installs over the vocabulary.
fn shift(salt: &[u8], position: usize) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in salt {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= position as u64;
    h.wrapping_mul(0x0000_0100_0000_01b3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_clean_and_complete() {
        let list = vocabulary();
        assert!(
            list.len() >= 7400,
            "the list came out short: {} entries",
            list.len()
        );
        for w in &list {
            assert!(
                w.chars().all(|c| c.is_ascii_lowercase()),
                "word with forbidden characters: {w}"
            );
        }
        let mut sorted = list.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), list.len(), "the list has repeated words");
    }

    #[test]
    fn respects_the_word_count() {
        let made = make(&Options {
            words: 4,
            separator: "-".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(made.value.split('-').count(), 4, "got: {}", made.value);
        assert_eq!(made.words, 4);
    }

    #[test]
    fn compatible_mode_is_alphanumeric_only() {
        for _ in 0..300 {
            let made = make(&Options {
                words: 5,
                compatible: true,
                symbol: true,
                digits: true,
                capitalize: true,
                separator: "/".into(),
                ..Default::default()
            })
            .unwrap();
            assert!(
                made.value.chars().all(|c| c.is_ascii_alphanumeric()),
                "compatible mode returned: {}",
                made.value
            );
            assert!(made.compatible);
        }
    }

    #[test]
    fn compatible_mode_ignores_a_bad_separator() {
        // The caller passed "/", which is refused on its own. In compatible mode
        // the separator is not used, so the call must still work.
        let made = make(&Options {
            words: 4,
            compatible: true,
            separator: "/".into(),
            ..Default::default()
        })
        .unwrap();
        assert!(!made.value.contains('/'));
    }

    #[test]
    fn refuses_an_unknown_separator() {
        for bad in ["/", "\\", ":", "|"] {
            assert!(
                make(&Options {
                    words: 4,
                    separator: bad.into(),
                    ..Default::default()
                })
                .is_err(),
                "should have rejected the separator {bad}"
            );
        }
    }

    #[test]
    fn refuses_absurd_word_counts() {
        for bad in [0usize, 1, 2, 13, 999] {
            assert!(
                make(&Options {
                    words: bad,
                    ..Default::default()
                })
                .is_err(),
                "should have rejected {bad} words"
            );
        }
    }

    #[test]
    fn entropy_matches_the_vocabulary() {
        let made = make(&Options {
            words: 6,
            ..Default::default()
        })
        .unwrap();
        let expected = 6.0 * (made.vocabulary as f64).log2();
        assert!((made.entropy_bits - expected).abs() < 0.001);
        assert!(
            made.entropy_bits > 76.0,
            "six words should clear 76 bits, got {}",
            made.entropy_bits
        );
    }

    #[test]
    fn digits_and_symbols_add_entropy_and_characters() {
        let plain = make(&Options {
            words: 4,
            ..Default::default()
        })
        .unwrap();
        let rich = make(&Options {
            words: 4,
            digits: true,
            symbol: true,
            ..Default::default()
        })
        .unwrap();
        assert!(rich.entropy_bits > plain.entropy_bits);
        // Not `rich.length > plain.length`: the words are drawn at random, and
        // four short words plus two characters can be shorter than four long
        // ones. The contract is the shape, not the rendered length.
        assert!(
            rich.value
                .chars()
                .any(|c| !c.is_alphanumeric() && c != '-'),
            "no symbol came out: {}",
            rich.value
        );
        let tail: String = rich.value.chars().rev().take(3).collect();
        let tail: String = tail.chars().rev().collect();
        assert!(
            tail.chars().take(2).all(|c| c.is_ascii_digit()),
            "the digits did not end up at the end: {tail}"
        );
    }

    #[test]
    fn capitalize_uppercases_the_first_letter_of_each_word() {
        let made = make(&Options {
            words: 5,
            capitalize: true,
            ..Default::default()
        })
        .unwrap();
        for word in made.value.split('-') {
            let first = word.chars().next().unwrap();
            assert!(
                first.is_ascii_uppercase(),
                "word without an initial capital: {word}"
            );
        }
    }

    #[test]
    fn two_calls_do_not_repeat() {
        let a = make(&Options::default()).unwrap().value;
        let b = make(&Options::default()).unwrap().value;
        assert_ne!(a, b, "two phrases in a row came out the same");
    }

    #[test]
    fn a_short_phrase_carries_advice() {
        let made = make(&Options {
            words: 3,
            separator: String::new(),
            ..Default::default()
        })
        .unwrap();
        assert!(made.advice.is_some(), "three words should warn");
    }

    #[test]
    fn the_random_block_adds_bits_and_stays_alphanumeric() {
        let plain = make(&Options {
            words: 4,
            ..Default::default()
        })
        .unwrap();
        let rich = make(&Options {
            words: 4,
            random_chars: 8,
            ..Default::default()
        })
        .unwrap();
        let expected = 8.0 * (ALNUM.len() as f64).log2();
        assert!(
            (rich.entropy_bits - plain.entropy_bits - expected).abs() < 0.001,
            "unexpected entropy delta"
        );
        assert_eq!(rich.random_chars, 8);
        assert!(
            rich.value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "the random block put something odd in: {}",
            rich.value
        );
    }

    #[test]
    fn the_salt_rotates_the_mapping_but_not_the_bits() {
        let salt = [7u8; 16];
        let other = [9u8; 16];
        assert_ne!(shift(&salt, 0), shift(&other, 0));
        // The same position with the same salt is stable...
        assert_eq!(shift(&salt, 3), shift(&salt, 3));
        // ...and different positions do not collapse to one value.
        assert_ne!(shift(&salt, 0), shift(&salt, 1));

        let plain = make_with(&Options::default(), &[]).unwrap();
        let salted = make_with(&Options::default(), &salt).unwrap();
        assert!((plain.entropy_bits - salted.entropy_bits).abs() < 0.001);
        assert_eq!(plain.words, salted.words);
    }

    #[test]
    fn refuses_an_absurd_random_block() {
        assert!(make(&Options {
            random_chars: MAX_RANDOM + 1,
            ..Default::default()
        })
        .is_err());
    }
}

/// The style a person picked, as it lives in `config.json`.
///
/// [`Options`] is what a request carries; this is what the file remembers. The
/// two stay separate so a stored default cannot be confused with an explicit
/// request, and so `configured` has somewhere to live.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PassphraseConfig {
    pub words: usize,
    pub separator: String,
    pub capitalize: bool,
    pub digits: bool,
    pub symbol: bool,
    /// Fresh characters per phrase. `#[serde(default)]` keeps a config written
    /// by 0.4.x (no field) loading as 0 instead of failing.
    #[serde(default)]
    pub random_chars: usize,
    pub compatible: bool,
    /// False until a person picks the style once. The widget asks the first
    /// time and never again; the defaults above are only a starting point.
    #[serde(default)]
    pub configured: bool,
}

impl Default for PassphraseConfig {
    fn default() -> Self {
        let o = Options::default();
        Self {
            words: o.words,
            separator: o.separator,
            capitalize: false,
            digits: false,
            symbol: false,
            // A fresh install gets a random block; an existing config without
            // the field keeps 0 (the old shape) until someone raises it.
            random_chars: 8,
            compatible: false,
            configured: false,
        }
    }
}

impl From<&PassphraseConfig> for Options {
    fn from(c: &PassphraseConfig) -> Self {
        Self {
            words: c.words,
            separator: c.separator.clone(),
            capitalize: c.capitalize,
            digits: c.digits,
            symbol: c.symbol,
            random_chars: c.random_chars,
            compatible: c.compatible,
        }
    }
}

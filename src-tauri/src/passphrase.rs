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
pub fn make(opts: &Options) -> Result<Made, String> {
    let list = vocabulary();
    let n = list.len();
    if n < 1000 {
        return Err(format!(
            "la lista de palabras no cargó bien: {n} entradas"
        ));
    }
    if opts.words < MIN_WORDS || opts.words > MAX_WORDS {
        return Err(format!(
            "cantidad de palabras fuera de rango: {} (se aceptan entre {MIN_WORDS} y {MAX_WORDS})",
            opts.words
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
            "separador no permitido: {separator:?}. Los válidos son {SEPARATORS:?}"
        ));
    }

    let mut picked: Vec<String> = Vec::with_capacity(opts.words);
    for _ in 0..opts.words {
        let idx = crate::rand::below(n as u32)? as usize;
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
    if opts.digits {
        for _ in 0..DIGITS {
            let d = crate::rand::below(10)? as u8;
            value.push(char::from(b'0' + d));
        }
    }
    if symbol {
        let i = crate::rand::below(SYMBOLS.len() as u32)? as usize;
        value.push(char::from(SYMBOLS[i]));
    }

    let bits_per_word = (n as f64).log2();
    let mut entropy_bits = (opts.words as f64) * bits_per_word;
    if opts.digits {
        entropy_bits += (DIGITS as f64) * 10f64.log2();
    }
    if symbol {
        entropy_bits += (SYMBOLS.len() as f64).log2();
    }

    let length = value.chars().count();
    let advice = if length < SHORT_WARNING {
        Some(format!(
            "Quedaron {length} caracteres. Hay sistemas que piden {SHORT_WARNING} o más: una palabra extra lo resuelve."
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
        advice,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_clean_and_complete() {
        let list = vocabulary();
        assert!(
            list.len() >= 7400,
            "la lista quedo corta: {} entradas",
            list.len()
        );
        for w in &list {
            assert!(
                w.chars().all(|c| c.is_ascii_lowercase()),
                "palabra con caracteres prohibidos: {w}"
            );
        }
        let mut sorted = list.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), list.len(), "la lista tiene palabras repetidas");
    }

    #[test]
    fn respects_the_word_count() {
        let made = make(&Options {
            words: 4,
            separator: "-".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(made.value.split('-').count(), 4, "quedo: {}", made.value);
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
                "modo compatible devolvio: {}",
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
                "debio rechazar el separador {bad}"
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
                "debio rechazar {bad} palabras"
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
            "seis palabras deberian pasar de 76 bits, dieron {}",
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
        assert!(rich.length > plain.length);
        let tail: String = rich.value.chars().rev().take(3).collect();
        let tail: String = tail.chars().rev().collect();
        assert!(
            tail.chars().take(2).all(|c| c.is_ascii_digit()),
            "los digitos no quedaron al final: {tail}"
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
                "palabra sin mayuscula inicial: {word}"
            );
        }
    }

    #[test]
    fn two_calls_do_not_repeat() {
        let a = make(&Options::default()).unwrap().value;
        let b = make(&Options::default()).unwrap().value;
        assert_ne!(a, b, "dos frases seguidas salieron iguales");
    }

    #[test]
    fn a_short_phrase_carries_advice() {
        let made = make(&Options {
            words: 3,
            separator: String::new(),
            ..Default::default()
        })
        .unwrap();
        assert!(made.advice.is_some(), "tres palabras deberian avisar");
    }
}

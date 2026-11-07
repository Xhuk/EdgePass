//! The only source of randomness in the app.
//!
//! Every credential edgePass invents — an MCP token, a passphrase — comes from
//! the operating system: `BCryptGenRandom` on Windows, `/dev/urandom` elsewhere.
//! Nothing is derived from the clock, the process id or a hashed string, so two
//! installs started in the same second do not produce the same values.

/// Fills `buf` with bytes from the operating system.
pub fn fill(buf: &mut [u8]) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Security::Cryptography::{
            BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        };
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status < 0 {
            return Err(format!("BCryptGenRandom failed: {status}"));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        use std::io::Read;
        let mut f = std::fs::File::open("/dev/urandom").map_err(|e| format!("/dev/urandom: {e}"))?;
        f.read_exact(buf).map_err(|e| format!("/dev/urandom: {e}"))
    }
}

/// A uniform integer in `0..n`, free of modulo bias.
///
/// `random % n` is faster and wrong: when `n` does not divide the range, the
/// first `range % n` values come up one time more often. Here a draw that lands
/// outside the largest multiple of `n` is thrown away. The retry costs almost
/// nothing and the distribution stays flat — which is the whole argument for a
/// passphrase: every word must be equally likely.
pub fn below(n: u32) -> Result<u32, String> {
    if n == 0 {
        return Err("rand::below(0) has no answer".into());
    }
    if n == 1 {
        return Ok(0);
    }
    let bound = u32::MAX - (u32::MAX % n);
    loop {
        let mut b = [0u8; 4];
        fill(&mut b)?;
        let v = u32::from_le_bytes(b);
        if v < bound {
            return Ok(v % n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_inside_the_range() {
        for n in [2u32, 7, 10, 7411, 7776] {
            for _ in 0..500 {
                let v = below(n).unwrap();
                assert!(v < n, "below({n}) returned {v}");
            }
        }
    }

    #[test]
    fn covers_the_whole_range() {
        // A coarse smoke test of the rejection loop: every slot of a small range
        // must appear. With modulo bias this still passes; the point is that it
        // never gets stuck or skips a slot.
        let n = 16u32;
        let mut seen = [0u32; 16];
        for _ in 0..4000 {
            seen[below(n).unwrap() as usize] += 1;
        }
        assert!(seen.iter().all(|c| *c > 0), "some slot never came up: {seen:?}");
    }

    #[test]
    fn refuses_zero() {
        assert!(below(0).is_err());
    }

    #[test]
    fn zero_is_the_only_answer_for_one() {
        assert_eq!(below(1).unwrap(), 0);
    }

    #[test]
    fn two_draws_differ() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        fill(&mut a).unwrap();
        fill(&mut b).unwrap();
        assert_ne!(a, b, "two fills in a row came out the same");
    }
}

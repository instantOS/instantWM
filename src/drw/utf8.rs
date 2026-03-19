//! UTF-8 decoding utilities.
//!
//! This is a direct port of the UTF-8 decoding logic from the original C `drw.c`.
//! It is intentionally minimal — just enough to walk a byte string codepoint-by-
//! codepoint so the text-rendering loop can dispatch each character to the
//! correct fallback font.

// ── Constants ─────────────────────────────────────────────────────────────────

/// The Unicode replacement character, returned for invalid sequences.
pub const UTF_INVALID: u32 = 0xFFFD;

/// Maximum byte length of a single UTF-8 encoded codepoint.
pub const UTF_SIZ: usize = 4;

/// Leading-byte values for each UTF-8 sequence length (index = length).
/// Index 0 is the continuation-byte pattern.
pub const UTFBYTE: [u8; UTF_SIZ + 1] = [0x80, 0x00, 0xC0, 0xE0, 0xF0];

/// Masks used to isolate the tag bits of each byte class.
pub const UTFMASK: [u8; UTF_SIZ + 1] = [0xC0, 0x80, 0xE0, 0xF0, 0xF8];

/// Minimum valid codepoint for each sequence length (guards against overlong sequences).
pub const UTFMIN: [u32; UTF_SIZ + 1] = [0, 0, 0x80, 0x800, 0x10000];

/// Maximum valid codepoint for each sequence length.
pub const UTFMAX: [u32; UTF_SIZ + 1] = [0x10FFFF, 0x7F, 0x7FF, 0xFFFF, 0x10FFFF];

// ── Public API ────────────────────────────────────────────────────────────────

/// Decode the first UTF-8 codepoint from `bytes`.
///
/// Returns `(byte_length, codepoint)`.
///
/// * If `bytes` is empty, returns `(0, UTF_INVALID)`.
/// * If the sequence is invalid or truncated, the returned length points past
///   the offending byte(s) so the caller can safely advance and continue.
pub fn utf8decode(bytes: &[u8]) -> (usize, u32) {
    if bytes.is_empty() {
        return (0, UTF_INVALID);
    }

    let len = utf8decode_byte(bytes[0]);

    if !(1..=UTF_SIZ).contains(&len) {
        return (1, UTF_INVALID);
    }

    if bytes.len() < len {
        return (0, UTF_INVALID);
    }

    // Strip the leading-byte tag bits to obtain the initial data bits.
    let mut codepoint = (bytes[0] as u32) & !(UTFMASK[len] as u32);

    for (i, &byte) in bytes.iter().skip(1).take(len - 1).enumerate() {
        let byte_class = utf8decode_byte(byte);
        if byte_class != 0 {
            return (i + 1, UTF_INVALID);
        }
        codepoint = (codepoint << 6) | ((byte as u32) & !(UTFMASK[0] as u32));
    }

    (len, utf8validate(codepoint, len))
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Identify the *class* of a byte:
/// * `0`  — continuation byte
/// * `1`  — single-byte (ASCII) sequence
/// * `2`  — 2-byte leading byte
/// * `3`  — 3-byte leading byte
/// * `4`  — 4-byte leading byte
fn utf8decode_byte(c: u8) -> usize {
    for i in 0..=UTF_SIZ {
        if (c & UTFMASK[i]) == UTFBYTE[i] {
            return i;
        }
    }
    0
}

/// Return `u` unchanged if it is a valid codepoint for a sequence of length
/// `i`, otherwise return [`UTF_INVALID`].
fn utf8validate(u: u32, i: usize) -> u32 {
    // Reject codepoints outside the valid range for this sequence length
    // (overlong encoding) and the surrogate pair range D800–DFFF.
    if !(UTFMIN[i]..=UTFMAX[i]).contains(&u) || (0xD800..=0xDFFF).contains(&u) {
        return UTF_INVALID;
    }
    u
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_roundtrip() {
        let (len, cp) = utf8decode(b"a");
        assert_eq!(len, 1);
        assert_eq!(cp, b'a' as u32);
    }

    #[test]
    fn two_byte_sequence() {
        // 'é' U+00E9 encodes as 0xC3 0xA9
        let (len, cp) = utf8decode("é".as_bytes());
        assert_eq!(len, 2);
        assert_eq!(cp, 0xE9);
    }

    #[test]
    fn three_byte_sequence() {
        // '中' U+4E2D encodes as 0xE4 0xB8 0xAD
        let (len, cp) = utf8decode("中".as_bytes());
        assert_eq!(len, 3);
        assert_eq!(cp, 0x4E2D);
    }

    #[test]
    fn four_byte_sequence() {
        // '𝄞' U+1D11E (musical symbol G clef) encodes as 4 bytes
        let (len, cp) = utf8decode("𝄞".as_bytes());
        assert_eq!(len, 4);
        assert_eq!(cp, 0x1D11E);
    }

    #[test]
    fn empty_input() {
        let (len, cp) = utf8decode(b"");
        assert_eq!(len, 0);
        assert_eq!(cp, UTF_INVALID);
    }

    #[test]
    fn invalid_leading_byte() {
        // 0xFF is never valid in UTF-8
        let (len, cp) = utf8decode(&[0xFF]);
        assert_eq!(len, 1);
        assert_eq!(cp, UTF_INVALID);
    }

    #[test]
    fn truncated_sequence() {
        // First byte of a 2-byte sequence with no continuation byte
        let (len, cp) = utf8decode(&[0xC3]);
        assert_eq!(len, 0);
        assert_eq!(cp, UTF_INVALID);
    }

    #[test]
    fn surrogate_rejected() {
        // U+D800 would encode as 0xED 0xA0 0x80 — must be rejected
        let (_, cp) = utf8decode(&[0xED, 0xA0, 0x80]);
        assert_eq!(cp, UTF_INVALID);
    }

    #[test]
    fn walks_string_correctly() {
        let s = "aé中";
        let bytes = s.as_bytes();
        let mut pos = 0;
        let mut codepoints = Vec::new();
        while pos < bytes.len() {
            let (len, cp) = utf8decode(&bytes[pos..]);
            assert!(len > 0, "infinite loop guard");
            codepoints.push(cp);
            pos += len;
        }
        assert_eq!(codepoints, vec![b'a' as u32, 0xE9, 0x4E2D]);
    }
}

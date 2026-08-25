//! Backend-independent classification of bar text into semantic font roles.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FontRole {
    Text,
    Icon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextRun<'a> {
    pub text: &'a str,
    pub role: FontRole,
}

/// Classify a Unicode scalar without consulting a backend font database.
///
/// Nerd Fonts allocate their patched icon sets in the Unicode private-use
/// areas. Powerline glyphs are part of that space too and intentionally share
/// the explicit icon face. Apart from Nerd Fonts' documented IEC additions,
/// ordinary Unicode symbols remain text so system fallback can choose the most
/// appropriate installed font.
pub(crate) fn role_for_char(ch: char) -> FontRole {
    if matches!(
        ch as u32,
        // Nerd Fonts' patched icon sets, plus its standardized IEC power
        // symbols which deliberately live outside private-use space.
        0xE000..=0xF8FF
            | 0xF0000..=0xFFFFD
            | 0x100000..=0x10FFFD
            | 0x23FB..=0x23FE
            | 0x2B58
    ) {
        FontRole::Icon
    } else {
        FontRole::Text
    }
}

pub(crate) fn runs(text: &str) -> Vec<TextRun<'_>> {
    let Some(first) = text.chars().next() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut start = 0;
    let mut role = role_for_char(first);
    for (index, ch) in text.char_indices().skip(1) {
        let next = role_for_char(ch);
        if next != role {
            result.push(TextRun {
                text: &text[start..index],
                role,
            });
            start = index;
            role = next;
        }
    }
    result.push(TextRun {
        text: &text[start..],
        role,
    });
    result
}

pub(crate) fn is_powerline_only(text: &str) -> bool {
    let mut saw_glyph = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        if !matches!(ch as u32, 0xE0A0..=0xE0D7) {
            return false;
        }
        saw_glyph = true;
    }
    saw_glyph
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_mixed_text_at_semantic_role_boundaries() {
        let classified = runs("bat \u{f240} 50%");
        assert_eq!(
            classified,
            [
                TextRun {
                    text: "bat ",
                    role: FontRole::Text
                },
                TextRun {
                    text: "\u{f240}",
                    role: FontRole::Icon
                },
                TextRun {
                    text: " 50%",
                    role: FontRole::Text
                },
            ]
        );
    }

    #[test]
    fn supplementary_private_use_is_an_icon_role() {
        assert_eq!(role_for_char('\u{f0001}'), FontRole::Icon);
    }

    #[test]
    fn nerd_font_iec_power_symbols_use_the_icon_role() {
        assert_eq!(role_for_char('\u{23fb}'), FontRole::Icon);
        assert_eq!(role_for_char('\u{2b58}'), FontRole::Icon);
    }

    #[test]
    fn powerline_detection_excludes_other_private_glyphs() {
        assert!(is_powerline_only(" \u{e0b0}"));
        assert!(!is_powerline_only("\u{f240}"));
        assert!(!is_powerline_only("A"));
    }
}

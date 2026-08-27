//! Shared typography policy for the top bar.
//!
//! Bar text mixes an ordinary text face with a larger Nerd Font icon face.
//! Correcting the icon size exposed a second problem: many patched glyphs have
//! asymmetric side bearings or ink outside their nominal advance, so adjacent
//! text could look lopsided or overlap the icon. Normal kerning cannot repair a
//! boundary between different faces.
//!
//! The policy is therefore:
//!
//! ```text
//! Text("bat") -- gap -- Icon("\u{f240}") -- gap -- Text("50%")
//! ```
//!
//! A single, text-size-relative gap is added at every direct text/icon
//! transition. A trailing icon safety gap is retained even when the next
//! character is whitespace: patched glyph ink can consume that whitespace's
//! advance before the following label begins. Adjacent icons remain flush, and
//! Powerline glyphs are excluded because their intentional ink overlap is what
//! makes segments tile seamlessly.
//!
//! Xft can insert the resulting integral pixel gap directly between font runs.
//! cosmic-text expresses the same policy as trailing span spacing; preserving
//! that spacing requires isolating the boundary grapheme and preventing it from
//! joining a ligature. These are different rasterizer adapters for one visual
//! rule, not separate sources of typography policy.

use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation;

use crate::bar::paint::TextOverflow;

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

/// A Nerd Fonts powerline arrow. These glyphs are drawn to tile seamlessly:
/// their ink extends past the nominal advance on purpose, so they must never
/// receive boundary padding or adjacent letters would be pushed away from
/// solid line segments.
pub(crate) fn is_powerline_glyph(ch: char) -> bool {
    matches!(ch as u32, 0xE0A0..=0xE0D7)
}

/// Breathing room around icon glyphs, expressed as a fraction of the *icon*
/// face size (or maximum face size). Patched-font icons are artificially
/// inflated relative to text and carry almost no side bearing of their own —
/// most even have zero-width bearings, and powerline arrows go slightly
/// negative — so explicit spacing is required wherever an icon directly
/// touches normal text.
const ICON_BOUNDARY_PAD_EM: f32 = 0.35;

/// Extra horizontal space (in pixels) inserted between a text glyph and an
/// adjacent icon glyph. Both sides of an icon get the same amount so the gaps
/// stay symmetric regardless of which fonts happen to sit next to it.
pub fn icon_boundary_pad_px(text_size: f32, icon_size: f32) -> u32 {
    let effective_size = icon_size.max(text_size).max(1.0);
    (effective_size * ICON_BOUNDARY_PAD_EM).round() as u32
}

/// Whether a text→icon transition needs padding before the first icon glyph.
///
/// Leading whitespace already separates a preceding label from the icon, and
/// powerline arrows tile through their neighbours, so neither case is padded.
/// Both arguments are validated for their roles so plain text pairs can never
/// synthesize a gap.
pub(crate) fn needs_gap_before_icon(prev: Option<char>, icon_first: char) -> bool {
    matches!(prev, Some(prev_char) if !prev_char.is_whitespace()
        && role_for_char(prev_char) == FontRole::Text)
        && role_for_char(icon_first) == FontRole::Icon
        && !is_powerline_glyph(icon_first)
}

/// Whether an icon→text transition needs padding after the last icon glyph.
///
/// Unlike the leading rule, whitespace remains a text neighbour here. Nerd
/// Font ink can consume its advance, so the icon needs its safety gap before
/// that whitespace. Adjacent icons and string edges remain gap-free.
pub(crate) fn needs_gap_after_icon(icon_last: char, next: Option<char>) -> bool {
    role_for_char(icon_last) == FontRole::Icon
        && !is_powerline_glyph(icon_last)
        && matches!(next, Some(next_char) if role_for_char(next_char) == FontRole::Text)
}

/// Whether a padding gap belongs *between* two adjacent characters.
///
/// Mirrors the two one-directional helpers above so render loops that walk a
/// flat character stream (the X11 run loop) can ask a single question.
pub(crate) fn boundary_gap_between(prev: char, next: char) -> bool {
    needs_gap_before_icon(Some(prev), next) || needs_gap_after_icon(prev, Some(next))
}

/// Letter spacing, in em units of a span shaped at `carrier_size`, whose
/// trailing advance equals one icon boundary pad measured from `icon_size.max(text_size)`.
///
/// cosmic-text implements letter spacing as an em-normalized *trailing*
/// advance per glyph, so a span rendered at the icon face size has to rescale
/// the value for both sides of an icon to cover the same pixel distance.
pub(crate) fn icon_gap_letter_spacing(carrier_size: f32, text_size: f32, icon_size: f32) -> f32 {
    let effective_size = icon_size.max(text_size).max(1.0);
    ICON_BOUNDARY_PAD_EM * effective_size / carrier_size.max(1.0)
}

/// Fit text to a cell using backend-provided advance measurement.
///
/// Clipped text is returned unchanged because both rasterizers already enforce
/// cell bounds. Ellipsis truncation occurs at extended grapheme boundaries and
/// uses a binary search over those boundaries, avoiding a shaped measurement
/// for every prefix. The returned text is shared policy; the backend only
/// rasterizes it. A borrowed value is returned when no fitting is needed.
pub(crate) fn fit_to_width<'a>(
    text: &'a str,
    max_width: i32,
    overflow: TextOverflow,
    mut measure: impl FnMut(&str) -> i32,
) -> Cow<'a, str> {
    let max_width = max_width.max(0);
    // Both rasterizers already clip to the cell bounds. Measuring and
    // rebuilding a clipped string only duplicates their work and, for
    // cosmic-text, would shape a series of throwaway prefixes.
    if text.is_empty() || overflow == TextOverflow::Clip {
        return Cow::Borrowed(text);
    }
    if measure(text) <= max_width {
        return Cow::Borrowed(text);
    }

    let suffix = "...";
    let suffix_width = measure(suffix);
    if suffix_width > max_width {
        return Cow::Borrowed("");
    }

    let grapheme_ends: Vec<_> = text
        .grapheme_indices(true)
        .map(|(start, grapheme)| start + grapheme.len())
        .collect();
    let mut low = 0;
    let mut high = grapheme_ends.len();
    let mut candidate = String::new();
    while low < high {
        let candidate_count = low + (high - low).div_ceil(2);
        let end = grapheme_ends[candidate_count - 1];
        candidate.clear();
        candidate.push_str(&text[..end]);
        candidate.push_str(suffix);
        if measure(&candidate) <= max_width {
            low = candidate_count;
        } else {
            high = candidate_count - 1;
        }
    }

    let fitted_end = low.checked_sub(1).map_or(0, |index| grapheme_ends[index]);
    let mut fitted = String::with_capacity(fitted_end + suffix.len());
    fitted.push_str(&text[..fitted_end]);
    fitted.push_str(suffix);
    Cow::Owned(fitted)
}

/// A [`TextRun`] annotated with the spacing its boundary cluster carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GappedRun<'a> {
    pub(crate) text: &'a str,
    pub(crate) role: FontRole,
    /// Optional letter spacing, in em units of this run's own face size,
    /// applied to the run's isolated boundary grapheme. `None` leaves shaping
    /// untouched.
    pub(crate) gap_em: Option<f32>,
    /// Prevent this span from joining a ligature that crosses the padded
    /// boundary. Cosmic Text attributes spacing to the start of a shaped
    /// cluster, so a boundary scalar absorbed into an earlier cluster would
    /// otherwise lose its gap.
    pub(crate) prevent_ligatures: bool,
}

/// Segment bar text into spans with symmetric icon boundary gaps baked in.
///
/// Each side of an inline icon contributes one [`ICON_BOUNDARY_PAD_EM_TEXT`]
/// worth of pixels: the text-side half trails the *last* glyph of the text
/// run, and the icon-side half trails the *last* glyph of the icon run, the
/// latter rescaled through [`icon_gap_letter_spacing`] so both halves measure
/// identical pixels. Boundary graphemes are isolated onto their own span, and
/// ligatures are disabled only in their boundary word, keeping the rest of
/// each run free of letter tracking and adjacent icons flush with each other.
/// Leading whitespace, string edges and powerline arrows stay unpadded;
/// trailing whitespace keeps the icon safety gap (see
/// [`needs_gap_before_icon`] / [`needs_gap_after_icon`]).
pub(crate) fn gapped_runs(text: &str, text_size: f32, icon_size: f32) -> Vec<GappedRun<'_>> {
    /// Push `run_text`, splitting off its final grapheme onto a gapped span
    /// when `gap_em` is set so combining sequences remain one shaped cluster.
    /// The rest of the boundary word also has ligatures disabled, preventing
    /// the final grapheme from being absorbed into a cluster that starts in
    /// the unspaced span.
    fn push_boundary_split<'a>(
        gapped: &mut Vec<GappedRun<'a>>,
        run_text: &'a str,
        role: FontRole,
        gap_em: Option<f32>,
    ) {
        let Some(gap) = gap_em else {
            gapped.push(GappedRun {
                text: run_text,
                role,
                gap_em: None,
                prevent_ligatures: false,
            });
            return;
        };

        let tail_start = run_text
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index);
        let boundary_word_start = run_text[..tail_start]
            .char_indices()
            .rev()
            .find_map(|(index, ch)| ch.is_whitespace().then_some(index + ch.len_utf8()))
            .unwrap_or(0);

        if boundary_word_start > 0 {
            gapped.push(GappedRun {
                text: &run_text[..boundary_word_start],
                role,
                gap_em: None,
                prevent_ligatures: false,
            });
        }
        if boundary_word_start < tail_start {
            gapped.push(GappedRun {
                text: &run_text[boundary_word_start..tail_start],
                role,
                gap_em: None,
                prevent_ligatures: true,
            });
        }
        gapped.push(GappedRun {
            text: &run_text[tail_start..],
            role,
            gap_em: Some(gap),
            prevent_ligatures: true,
        });
    }

    let classified = runs(text);
    let mut gapped = Vec::with_capacity(classified.len());
    for index in 0..classified.len() {
        let run = &classified[index];
        // Runs strictly alternate roles, so only the following neighbour can
        // close a gap on this run's trailing glyph.
        let gap_em = classified
            .get(index + 1)
            .and_then(|next| {
                let run_last = run.text.chars().next_back()?;
                let next_first = next.text.chars().next()?;
                Some(match run.role {
                    FontRole::Icon => needs_gap_after_icon(run_last, Some(next_first))
                        .then(|| icon_gap_letter_spacing(icon_size, text_size, icon_size)),
                    FontRole::Text => needs_gap_before_icon(Some(run_last), next_first)
                        .then(|| icon_gap_letter_spacing(text_size, text_size, icon_size)),
                })
            })
            .flatten();
        push_boundary_split(&mut gapped, run.text, run.role, gap_em);
    }
    gapped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monospace_width(text: &str) -> i32 {
        text.graphemes(true).count() as i32
    }

    #[test]
    fn fitting_keeps_text_borrowed_when_it_already_fits() {
        let fitted = fit_to_width("hello", 5, TextOverflow::Ellipsis, monospace_width);
        assert!(matches!(fitted, Cow::Borrowed("hello")));
    }

    #[test]
    fn fitting_applies_one_shared_ellipsis_policy() {
        assert_eq!(
            fit_to_width("abcdef", 5, TextOverflow::Ellipsis, monospace_width),
            "ab..."
        );
        assert_eq!(
            fit_to_width("abcdef", 5, TextOverflow::Clip, monospace_width),
            "abcdef"
        );
        assert_eq!(
            fit_to_width("abcdef", 2, TextOverflow::Ellipsis, monospace_width),
            ""
        );
    }

    #[test]
    fn clipping_is_deferred_to_the_rasterizer_without_measuring() {
        let mut measurements = 0;
        let fitted = fit_to_width("e\u{301}x", 1, TextOverflow::Clip, |_| {
            measurements += 1;
            99
        });
        assert!(matches!(fitted, Cow::Borrowed("e\u{301}x")));
        assert_eq!(measurements, 0);
    }

    #[test]
    fn ellipsis_never_splits_a_grapheme_cluster() {
        assert_eq!(
            fit_to_width("e\u{301}xyzz", 4, TextOverflow::Ellipsis, monospace_width),
            "e\u{301}..."
        );
    }

    #[test]
    fn ellipsis_uses_logarithmically_many_measurements() {
        let text = "x".repeat(1024);
        let mut measurements = 0;
        let fitted = fit_to_width(&text, 100, TextOverflow::Ellipsis, |candidate| {
            measurements += 1;
            candidate.len() as i32
        });
        assert_eq!(fitted.len(), 100);
        assert!(measurements <= 13, "used {measurements} measurements");
    }

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

    #[test]
    fn icon_boundaries_get_padded_but_edges_do_not() {
        let icon = '\u{f240}';

        // Icon in the middle of text: padded on both sides.
        assert!(needs_gap_before_icon(Some('t'), icon));
        assert!(needs_gap_after_icon(icon, Some('5')));

        // Start/end of string: nothing to pad away from.
        assert!(!needs_gap_before_icon(None, icon));
        assert!(!needs_gap_after_icon(icon, None));

        // Leading whitespace separates on its own; trailing whitespace keeps
        // the safety advance before the following label.
        assert!(!needs_gap_before_icon(Some(' '), icon));
        assert!(needs_gap_after_icon(icon, Some(' ')));

        // An icon next to another icon stays flush.
        let other_icon = '\u{f015}';
        assert!(!needs_gap_before_icon(Some(other_icon), icon));
        assert!(!needs_gap_after_icon(icon, Some(other_icon)));
    }

    #[test]
    fn powerline_arrows_never_receive_boundary_padding() {
        // Arrows intentionally paint outside their advance to tile segments.
        for arrow in ['\u{e0b0}', '\u{e0b2}'] {
            assert!(!needs_gap_before_icon(Some('t'), arrow));
            assert!(!needs_gap_after_icon(arrow, Some('5')));
            assert!(is_powerline_glyph(arrow));
            assert!(!is_powerline_glyph('\u{f240}'));
        }
    }

    #[test]
    fn boundary_padding_scales_with_inflated_icon_size() {
        assert_eq!(icon_boundary_pad_px(12.0, 16.0), 6);
        assert_eq!(icon_boundary_pad_px(12.0, 12.0), 4);
        assert_eq!(icon_boundary_pad_px(14.0, 18.0), 6);
        assert_eq!(icon_boundary_pad_px(20.0, 26.0), 9);
        assert_eq!(icon_boundary_pad_px(1.0, 1.0), 0);
    }

    #[test]
    fn gap_letter_spacing_scales_to_the_carrier_face_size() {
        // When carrier is text (12px) with inflated icon (16px), gap scales to 16px…
        assert!((icon_gap_letter_spacing(12.0, 12.0, 16.0) - (0.35 * 16.0 / 12.0)).abs() < 1e-6);
        // …while on the icon face (16px) it uses the base fraction.
        assert!((icon_gap_letter_spacing(16.0, 12.0, 16.0) - 0.35).abs() < 1e-6);
        // Degenerate carrier sizes still produce a finite value.
        assert!((icon_gap_letter_spacing(0.0, 12.0, 16.0) - 5.6).abs() < 1e-5);
    }

    #[test]
    fn xft_and_cosmic_spacing_resolve_to_the_same_device_pixel() {
        for (text_size, icon_size) in [
            (10.0, 14.0),
            (12.0, 16.0),
            (14.0, 18.0),
            (16.0, 20.0),
            (20.0, 26.0),
        ] {
            let cosmic_pixels_text =
                icon_gap_letter_spacing(text_size, text_size, icon_size) * text_size;
            assert_eq!(
                cosmic_pixels_text.round() as u32,
                icon_boundary_pad_px(text_size, icon_size)
            );
            let cosmic_pixels_icon =
                icon_gap_letter_spacing(icon_size, text_size, icon_size) * icon_size;
            assert_eq!(
                cosmic_pixels_icon.round() as u32,
                icon_boundary_pad_px(text_size, icon_size)
            );
        }
    }

    #[test]
    fn gapped_runs_pads_both_sides_of_an_inline_icon() {
        let segments = gapped_runs("ab\u{f240}cd", 12.0, 16.0);
        assert_eq!(segments.len(), 4);

        assert_eq!(segments[0].text, "a");
        assert_eq!(segments[0].role, FontRole::Text);
        assert_eq!(segments[0].gap_em, None);
        // The glyph before the icon carries the text-side half of the gap.
        assert_eq!(
            segments[1],
            GappedRun {
                text: "b",
                role: FontRole::Text,
                gap_em: Some(icon_gap_letter_spacing(12.0, 12.0, 16.0)),
                prevent_ligatures: true,
            }
        );
        // The icon itself carries the icon-side half.
        assert_eq!(
            segments[2],
            GappedRun {
                text: "\u{f240}",
                role: FontRole::Icon,
                gap_em: Some(icon_gap_letter_spacing(16.0, 12.0, 16.0)),
                prevent_ligatures: true,
            }
        );
        // Trailing text has nothing to pad away from, so it keeps its run
        // whole instead of splitting off a lone boundary glyph.
        assert_eq!(
            segments[3],
            GappedRun {
                text: "cd",
                role: FontRole::Text,
                gap_em: None,
                prevent_ligatures: false,
            }
        );
    }

    #[test]
    fn gapped_runs_preserve_trailing_icon_safety_through_whitespace() {
        // A nested fn avoids the closure-lifetime knot around borrowing `text`.
        fn flatten(text: &str) -> Vec<(&str, Option<f32>)> {
            gapped_runs(text, 12.0, 16.0)
                .iter()
                .map(|segment| (segment.text, segment.gap_em))
                .collect()
        }

        // Leading whitespace needs no extra gap, but the icon keeps its
        // trailing safety advance before the following whitespace.
        assert_eq!(
            flatten("a \u{f240} b"),
            [
                ("a ", None),
                ("\u{f240}", Some(icon_gap_letter_spacing(16.0, 12.0, 16.0))),
                (" b", None)
            ],
        );

        // Powerline arrows tile through their neighbours.
        assert_eq!(
            flatten("a\u{e0b0}b"),
            [("a", None), ("\u{e0b0}", None), ("b", None)],
        );

        // The leading string edge adds nothing, while the icon's trailing
        // safety advance survives the space used by status generators.
        assert_eq!(
            flatten("\u{f240} 50%"),
            [
                ("\u{f240}", Some(icon_gap_letter_spacing(16.0, 12.0, 16.0))),
                (" 50%", None)
            ],
        );
        // An edge only ever suppresses the half facing outward: a trailing
        // icon loses its own half while the text-side half still rides "t".
        assert_eq!(
            flatten("bat\u{f240}"),
            [
                ("ba", None),
                ("t", Some(icon_gap_letter_spacing(12.0, 12.0, 16.0))),
                ("\u{f240}", None),
            ],
        );
        assert_eq!(flatten(""), []);

        // Adjacent icons share one run and stay flush end to end…
        assert_eq!(flatten("\u{f240}\u{f015}"), [("\u{f240}\u{f015}", None)]);

        // …and an icon trailing real text keeps the gap against that text.
        assert_eq!(
            flatten("ab\u{f240}"),
            [
                ("a", None),
                ("b", Some(icon_gap_letter_spacing(12.0, 12.0, 16.0))),
                ("\u{f240}", None),
            ],
        );
    }

    #[test]
    fn only_the_boundary_grapheme_of_each_run_gets_letter_spacing() {
        let segments = gapped_runs("abc\u{f240}\u{f015}de", 12.0, 16.0);
        assert_eq!(
            segments,
            [
                GappedRun {
                    text: "ab",
                    role: FontRole::Text,
                    gap_em: None,
                    prevent_ligatures: true,
                },
                GappedRun {
                    text: "c",
                    role: FontRole::Text,
                    gap_em: Some(icon_gap_letter_spacing(12.0, 12.0, 16.0)),
                    prevent_ligatures: true,
                },
                GappedRun {
                    text: "\u{f240}",
                    role: FontRole::Icon,
                    gap_em: None,
                    prevent_ligatures: true,
                },
                GappedRun {
                    text: "\u{f015}",
                    role: FontRole::Icon,
                    gap_em: Some(icon_gap_letter_spacing(16.0, 12.0, 16.0)),
                    prevent_ligatures: true,
                },
                GappedRun {
                    text: "de",
                    role: FontRole::Text,
                    gap_em: None,
                    prevent_ligatures: false,
                },
            ]
        );
    }

    #[test]
    fn multibyte_boundaries_split_on_grapheme_boundaries() {
        let segments = gapped_runs("déjà\u{f240}", 12.0, 16.0);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "déj");
        assert_eq!(segments[1].text, "à");
        assert_eq!(segments[1].role, FontRole::Text);
        assert!(segments[1].gap_em.is_some());
        assert_eq!(segments[2].role, FontRole::Icon);
        assert_eq!(segments[2].gap_em, None);
    }

    #[test]
    fn battery_status_keeps_safety_advance_before_percentage() {
        let segments = gapped_runs(" \u{f244} 87% ", 12.0, 16.0);
        let battery = segments
            .iter()
            .find(|segment| segment.text == "\u{f244}")
            .expect("battery icon segment");

        assert_eq!(battery.role, FontRole::Icon);
        assert_eq!(battery.gap_em, Some(icon_gap_letter_spacing(16.0, 12.0, 16.0)));
    }

    #[test]
    fn combining_sequence_carries_the_boundary_gap_as_one_cluster() {
        let segments = gapped_runs("e\u{301}\u{f240}", 12.0, 16.0);

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "e\u{301}");
        assert_eq!(segments[0].role, FontRole::Text);
        assert!(segments[0].gap_em.is_some());
        assert!(segments[0].prevent_ligatures);
    }

    #[test]
    fn boundary_word_is_marked_to_prevent_ligature_clusters() {
        let segments = gapped_runs("label fi\u{f240}", 12.0, 16.0);

        assert_eq!(segments[0].text, "label ");
        assert!(!segments[0].prevent_ligatures);
        assert_eq!(segments[1].text, "f");
        assert_eq!(segments[1].gap_em, None);
        assert!(segments[1].prevent_ligatures);
        assert_eq!(segments[2].text, "i");
        assert!(segments[2].gap_em.is_some());
        assert!(segments[2].prevent_ligatures);
    }

    #[test]
    fn boundary_gap_between_answers_one_question_per_transition() {
        let icon = '\u{f240}';
        assert!(boundary_gap_between('t', icon));
        assert!(boundary_gap_between(icon, 't'));
        // Leading whitespace separates on its own; trailing whitespace keeps
        // the icon's safety advance. Icons stay flush together.
        assert!(!boundary_gap_between(' ', icon));
        assert!(boundary_gap_between(icon, ' '));
        assert!(!boundary_gap_between(icon, '\u{f015}'));
        // Powerline arrows tile; plain adjacent letters need no pad.
        assert!(!boundary_gap_between('\u{e0b0}', 't'));
        assert!(!boundary_gap_between('t', 'x'));
    }
}

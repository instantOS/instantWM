//! Tag name management.
//!
//! Tags can be given custom names at runtime (e.g. by a status bar script or
//! a keybinding).  The two functions here cover the full lifecycle:
//!
//! * [`name_tag`]       – rename the currently active tag(s).
//! * [`reset_name_tag`] – restore all tag names to their defaults (`"1"`…`"9"`).

use crate::bar::draw_bars;
use crate::contexts::WmCtx;
use crate::tags::bar::get_tag_width;
use crate::types::MAX_TAGS;

/// Maximum byte-length (excluding the NUL terminator) accepted for a tag name.
const MAX_TAGLEN: usize = 16;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Rename the currently visible tag(s).
///
/// If the string is empty, the tag name is reset to its default
/// (`"1"` … `"9"`).  Names longer than [`MAX_TAGLEN`] bytes are silently
/// ignored.
///
/// All tags included in the monitor's current tagset are renamed, so the
/// function works correctly even when multiple tags are visible at once.
pub fn name_tag(ctx: &mut WmCtx, arg: &str) {
    if arg.len() >= MAX_TAGLEN {
        return;
    }

    // -----------------------------------------------------------------------
    // 2. Find which tags are currently selected on the active monitor.
    // -----------------------------------------------------------------------
    let (numtags, tagset) = (
        ctx.g.tags.count(),
        ctx.g
            .monitors
            .get(ctx.g.selmon)
            .map(|m| m.tagset[m.seltags as usize])
            .unwrap_or(0),
    );

    if tagset == 0 {
        return;
    }

    // -----------------------------------------------------------------------
    // 3. Apply the new (or default) name to every tag in the current tagset.
    // -----------------------------------------------------------------------
    for i in 0..numtags.min(MAX_TAGS) {
        if (tagset & (1 << i)) == 0 {
            continue;
        }
        if let Some(tag) = ctx.g.tags.tags.get_mut(i) {
            if !arg.is_empty() {
                tag.name = arg.to_string();
            } else {
                tag.name = default_tag_name(i);
            }
        }
    }

    ctx.g.tags.width = get_tag_width(ctx);
    draw_bars(ctx);
}

/// Reset every tag's name back to its default (`"1"` … `"9"`, etc.).
pub fn reset_name_tag(ctx: &mut WmCtx) {
    let count = ctx.g.tags.count().min(MAX_TAGS);

    for i in 0..count {
        if let Some(tag) = ctx.g.tags.tags.get_mut(i) {
            tag.name = default_tag_name(i);
        }
    }

    ctx.g.tags.width = get_tag_width(ctx);
    draw_bars(ctx);
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Return the default display name for tag index `i` (0-based).
///
/// Tags 0–7 → `"1"`…`"8"`, tag 8 → `"9"`.
fn default_tag_name(i: usize) -> String {
    if i == 8 {
        "9".to_string()
    } else {
        ((b'1' + i as u8) as char).to_string()
    }
}

//! Tag name management.

use crate::bar::draw_bars;
use crate::contexts::WmCtx;
use crate::tags::bar::get_tag_width;
use crate::types::MAX_TAGS;

/// Maximum byte-length (excluding the NUL terminator) accepted for a tag name.
const MAX_TAGLEN: usize = 16;

/// Rename the currently visible tag(s) on the selected monitor.
///
/// If the string is empty, the tag name is reset to its default (`"1"` … `"9"`).
/// Names longer than [`MAX_TAGLEN`] bytes are silently ignored.
///
/// All tags included in the monitor's current tagset are renamed, so the
/// function works correctly even when multiple tags are visible at once.
pub fn name_tag(ctx: &mut WmCtx, arg: &str) {
    if arg.len() >= MAX_TAGLEN {
        return;
    }

    let mon = ctx.g.selected_monitor();
    let (numtags, tagset) = (mon.tags.len(), mon.selected_tags());

    if tagset == 0 {
        return;
    }

    // Apply the new (or default) name to every tag in the current tagset
    // on every monitor, so secondary monitors stay in sync.
    for (_idx, mon) in ctx.g.monitors.iter_mut() {
        for i in 0..numtags.min(MAX_TAGS) {
            if (tagset & (1 << i)) == 0 {
                continue;
            }
            if let Some(tag) = mon.tags.get_mut(i) {
                tag.name = if !arg.is_empty() {
                    arg.to_string()
                } else {
                    default_tag_name(i)
                };
            }
        }
    }

    ctx.g.tags.width = get_tag_width(ctx);
    draw_bars(ctx);
}

/// Reset every tag's name back to its default (`"1"` … `"9"`, etc.) on all monitors.
pub fn reset_name_tag(ctx: &mut WmCtx) {
    let num_tags = ctx.g.tags.num_tags.min(MAX_TAGS);
    for (_idx, mon) in ctx.g.monitors.iter_mut() {
        for i in 0..num_tags {
            if let Some(tag) = mon.tags.get_mut(i) {
                tag.name = default_tag_name(i);
            }
        }
    }

    ctx.g.tags.width = get_tag_width(ctx);
    draw_bars(ctx);
}

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

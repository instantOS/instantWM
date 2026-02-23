//! Tag name management.
//!
//! Tags can be given custom names at runtime (e.g. by a status bar script or
//! a keybinding).  The two functions here cover the full lifecycle:
//!
//! * [`name_tag`]       – rename the currently active tag(s).
//! * [`reset_name_tag`] – restore all tag names to their defaults (`"1"`…`"9"`).

use crate::bar::draw_bars;
use crate::globals::{get_globals, get_globals_mut};
use crate::tags::bar::get_tag_width;
use crate::types::{Arg, MAX_TAGS};

/// Maximum byte-length (excluding the NUL terminator) accepted for a tag name.
const MAX_TAGLEN: usize = 16;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Rename the currently visible tag(s).
///
/// The new name is read from `arg.v` as a C string pointer.  If the pointer
/// is `None`, or the string is empty, the tag name is reset to its default
/// (`"1"` … `"9"`).  Names longer than [`MAX_TAGLEN`] bytes are silently
/// ignored.
///
/// All tags included in the monitor's current tagset are renamed, so the
/// function works correctly even when multiple tags are visible at once.
///
/// # Safety
/// `arg.v` must either be `None` or a valid pointer to a NUL-terminated C
/// string that remains live for the duration of this call.
pub fn name_tag(arg: &Arg) {
    // -----------------------------------------------------------------------
    // 1. Decode the name from the raw pointer.
    // -----------------------------------------------------------------------
    let name_ptr = arg.v;
    let name_bytes = match name_ptr {
        Some(ptr) => {
            let cstr = unsafe { std::ffi::CStr::from_ptr(ptr as *const i8) };
            cstr.to_bytes()
        }
        None => b"" as &[u8],
    };

    if name_bytes.len() >= MAX_TAGLEN {
        return;
    }

    // -----------------------------------------------------------------------
    // 2. Find which tags are currently selected on the active monitor.
    // -----------------------------------------------------------------------
    let (numtags, tagset) = {
        let globals = get_globals();
        let numtags = globals.tags.count();
        let tagset = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.tagset[m.seltags as usize])
            .unwrap_or(0);
        (numtags, tagset)
    };

    if tagset == 0 {
        return;
    }

    // -----------------------------------------------------------------------
    // 3. Apply the new (or default) name to every tag in the current tagset.
    // -----------------------------------------------------------------------
    let globals = get_globals_mut();
    for i in 0..numtags.min(MAX_TAGS) {
        if (tagset & (1 << i)) == 0 {
            continue;
        }
        if let Some(tag) = globals.tags.tags.get_mut(i) {
            if !name_bytes.is_empty() {
                tag.name = String::from_utf8_lossy(name_bytes).into_owned();
            } else {
                tag.name = default_tag_name(i);
            }
        }
    }

    globals.tags.width = get_tag_width();
    draw_bars();
}

/// Reset every tag's name back to its default (`"1"` … `"9"`, etc.).
pub fn reset_name_tag(_arg: &Arg) {
    let globals = get_globals_mut();
    let count = globals.tags.count().min(MAX_TAGS);

    for i in 0..count {
        if let Some(tag) = globals.tags.tags.get_mut(i) {
            tag.name = default_tag_name(i);
        }
    }

    globals.tags.width = get_tag_width();
    draw_bars();
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

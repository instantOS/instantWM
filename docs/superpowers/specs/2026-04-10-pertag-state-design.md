# Per-Tag-Mask State: nmaster, mfact, showbar, layouts

**Date:** 2026-04-10
**Status:** Draft

## Overview

State that should follow the *combination* of tags active (not just a single focused tag) moves into a `HashMap<u32, PertagState>` keyed by `selected_tags().bits()`. This uses the same sparse/on-demand pattern already used by `tag_focus_history`. A newly encountered mask initializes with hardcoded defaults; subsequent visits restore the previously stored values.

**Values migrated:** `nmaster`, `mfact`, `showbar`, `TagLayouts`

**Values kept per-tag (in `Vec<TagNames>`):** `name`, `alt_name`

---

## New Types

### `PertagState` (`src/types/monitor.rs`)

```rust
/// Runtime state restored when a tag mask is revisited.
#[derive(Debug, Clone, Default)]
pub struct PertagState {
    pub nmaster: i32,
    pub mfact: f32,
    pub showbar: bool,
    pub layouts: TagLayouts,
}
```

Initial defaults match the old `Tag::default()` values:
- `nmaster: 1`
- `mfact: 0.55`
- `showbar: true`
- `layouts: TagLayouts::default()` (Tile primary, Floating secondary, no last_layout)

### `TagNames` (replaces `Vec<Tag>` for name-only storage)

```rust
/// Per-tag name/color data. No runtime layout state.
#[derive(Debug, Clone, Default)]
pub struct TagNames {
    pub name: String,
    pub alt_name: String,
}
```

---

## `Monitor` Struct Changes

**Removed fields:**
- `nmaster: i32`
- `mfact: f32`
- `tags: Vec<Tag>`

**Added fields:**
- `tags: Vec<TagNames>` — name/alt_name only
- `pertag: HashMap<u32, PertagState>` — keyed by `selected_tags().bits()`

**Removed methods (replaced):**
- `current_tag()` / `current_tag_mut()` — replaced by `pertag_state()` accessor
- `shows_bar()` — reads `pertag_state().showbar`
- `current_layout()` — reads `pertag_state().layouts.get_layout()`
- `layout_symbol()` / `is_tiling_layout()` / `is_monocle_layout()` / `toggle_layout_slot()` — via pertag
- `init_tags(template: &[Tag])` — signature changes to `&[TagNames]`

**Added methods:**
```rust
impl Monitor {
    /// Get or initialize state for the current tag mask.
    pub fn pertag_state(&mut self) -> &mut PertagState {
        let mask = self.selected_tags().bits();
        self.pertag.entry(mask).or_insert(PertagState::default())
    }

    /// Get a mutable reference to the names for a given tag index (1-based).
    pub fn tag_name(&self, tag_index: usize) -> Option<&TagNames> {
        tag_index.checked_sub(1).and_then(|i| self.tags.get(i))
    }
}
```

---

## Call Sites to Update

### Deletions

| Location | Reason |
|---|---|
| `tags/view.rs:332` `apply_pertag_settings()` | No longer needed — state lives in pertag map |
| `Monitor::nmaster` field | Replaced by `pertag_state().nmaster` |
| `Monitor::mfact` field | Replaced by `pertag_state().mfact` |
| `Tag::nmaster`, `Tag::mfact`, `Tag::showbar`, `Tag::layouts` fields | Replaced by pertag map |
| `TagLayouts` init in `Monitor::init_tags()` | No longer per-tag |

### Updates

| Location | Old | New |
|---|---|---|
| `layouts/manager.rs:310` `inc_nmaster_by()` | `m.nmaster = ...` then `m.tags[tag-1].nmaster = ...` | `m.pertag_state().nmaster = ...` |
| `layouts/manager.rs:332` `set_mfact()` | `m.mfact = ...` then `m.tags[tag-1].mfact = ...` | `m.pertag_state().mfact = ...` |
| `layouts/manager.rs:244` `set_layout()` | `m.tags[tag-1].layouts.set_layout(layout)` | `m.pertag_state().layouts.set_layout(layout)` |
| `layouts/manager.rs:254` `toggle_layout()` | `m.tags[tag-1].layouts.toggle_slot()` | `m.pertag_state().layouts.toggle_slot()` |
| `toggles.rs:161` bar toggle | `selmon.tags[current_tag-1].showbar = selmon.showbar` | `selmon.pertag_state().showbar = selmon.showbar` |
| `Monitor::shows_bar()` | `current_tag().map(\|t\| t.showbar)` | `self.pertag_state().showbar` |
| `Monitor::current_layout()` | `current_tag().map(\|t\| t.layouts.get_layout())` | `self.pertag_state().layouts.get_layout()` |
| `Monitor::toggle_layout_slot()` | `current_tag_mut().map(\|t\| t.layouts.toggle_slot())` | `self.pertag_state().layouts.toggle_slot()` |
| `bar/paint.rs` layout symbol | `mon.current_tag().map(\|t\| t.layouts.symbol())` | `mon.pertag_state().layouts.symbol()` |
| `Monitor::layout_symbol()` | via `current_tag()` | via `pertag_state()` |
| `Monitor::is_tiling_layout()` | via `current_tag()` | via `pertag_state()` |
| `Monitor::is_monocle_layout()` | via `current_tag()` | via `pertag_state()` |
| Tag initialization | `init_tags(&[Tag])` | `init_tags(&[TagNames])` |

### `tag_focus_history` — unchanged
`HashMap<u32, WindowId>` already uses the same key scheme. No changes needed.

---

## IPC Changes

`src/ipc/tag.rs` returns tag state over IPC. Change reads from `Tag` fields to `pertag_state()` for nmaster/mfact/layouts. Names still come from `Vec<TagNames>`.

---

## Edge Cases

### First visit to a mask
`pertag.entry(mask).or_insert(PertagState::default())` initializes with hardcoded defaults (1, 0.55, true, Tile/Floating).

### `current_tag` is `None` (multi-tag view)
`selected_tags().bits()` is a multi-bit mask. The same entry is shared by all selected tags — no per-tag splitting. This matches the existing `tag_focus_history` behavior.

### Empty tag set (`selected_tags().bits() == 0`)
Not expected to occur in practice (tag 0 is scratchpad-only and never selected alone). If it does, the empty mask key `0` would be used — a potential no-op bucket. Acceptable for now.

### Bar visibility toggle
The existing toggle in `toggles.rs` inverts `selmon.showbar` and applies it to `selmon.tags[current_tag-1].showbar`. After the change it writes to `selmon.pertag_state().showbar` instead.

---

## Scope for Single Implementation Plan

This design covers one refactor pass: introducing `PertagState`, replacing `Vec<Tag>` with `Vec<TagNames>`, updating all call sites, deleting `apply_pertag_settings()`. No new features. No layout algorithm changes. No tag creation/destruction logic changes.

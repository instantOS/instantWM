# Per-Tag-Mask State Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace per-tag storage of `nmaster`, `mfact`, `showbar`, and `layouts` with per-tag-mask storage in a `HashMap<u32, PertagState>` on `Monitor`, keyed by `selected_tags().bits()`.

**Architecture:** `Vec<Tag>` becomes `Vec<TagNames>` (name/alt_name only). Runtime state moves to a sparse `HashMap<u32, PertagState>` using the same key scheme as `tag_focus_history`. Layout algorithms read `monitor.nmaster`/`monitor.mfact` directly from the pertag entry during layout; callers sync these back after.

**Tech Stack:** Rust, no new dependencies.

---

## File Map

```
src/types/monitor.rs     — PertagState, TagNames, Monitor changes, pertag_state() helper
src/types/tag.rs        — Remove nmaster/mfact/showbar/layouts from Tag
src/globals.rs          — tag_template: Vec<Tag> → Vec<TagNames>; build_tag_template returns Vec<TagNames>
src/layouts/algo/tile.rs     — Direct nmaster writes (clamp to client count)
src/layouts/algo/stack.rs    — Direct nmaster/mfact writes
src/layouts/algo/three_column.rs — Direct mfact write
src/layouts/manager.rs  — set_layout, toggle_layout, inc_nmaster_by, set_mfact, sync_pertag
src/toggles.rs          — bar toggle (pertag_state())
src/tags/view.rs        — delete apply_pertag_settings()
src/monitor.rs          — init_tags signature → &[TagNames]; update callers
src/floating/overlay.rs    — calculate_yoffset: add mask param
src/floating/movement.rs   — snap_window: add mask param
src/floating/snap.rs       — snap_monitor: add mask param
src/mouse/drag/move_drop.rs — read showbar from pertag
src/mouse/hover.rs          — read showbar from pertag
src/layouts/algo/overview.rs — read showbar from pertag
src/systray/x11.rs          — read showbar from pertag
src/client/rules.rs         — read showbar from pertag
src/backend/x11/events/handlers.rs — read showbar from pertag
src/bar/paint.rs            — layout_symbol via pertag_state()
src/bar/scene.rs            — layout_symbol via pertag_state()
src/bar/widgets.rs         — layout_symbol via pertag_state()
```

---

## Task 1: Define `PertagState` and `TagNames` types

**Files:**
- Modify: `src/types/monitor.rs`

Add at the top of `src/types/monitor.rs` (after the existing imports) if not already present:
```rust
use std::collections::HashMap;
```

Add at the bottom of `src/types/monitor.rs` (after the `Monitor` impl block, before the tests):

```rust
/// Runtime state restored when a tag mask is revisited.
/// Initialized with hardcoded defaults on first visit.
#[derive(Debug, Clone, Default)]
pub struct PertagState {
    pub nmaster: i32,
    pub mfact: f32,
    pub showbar: bool,
    pub layouts: TagLayouts,
}

/// Per-tag name data. No runtime layout state.
#[derive(Debug, Clone, Default)]
pub struct TagNames {
    pub name: String,
    pub alt_name: String,
}
```

- [ ] **Step 1: Add HashMap import**

Verify `use std::collections::HashMap;` is at the top of the file.

- [ ] **Step 2: Add PertagState and TagNames structs**

Add both structs at the bottom of the file.

- [ ] **Step 3: Verify build**

Run: `cargo build 2>&1 | head -30`
Expected: compiles (only new types added)

- [ ] **Step 4: Commit**

```bash
git add src/types/monitor.rs
git commit -m "feat: add PertagState and TagNames types"
```

---

## Task 2: Add `pertag_state()` helper and `pertag` field to `Monitor`

**Files:**
- Modify: `src/types/monitor.rs`

Change the `Monitor` struct:
- Remove: `pub mfact: f32`, `pub nmaster: i32`
- Add: `pub pertag: HashMap<u32, PertagState>`

Update `Monitor::default()`:
- Remove `mfact: 0.55, nmaster: 1,`
- Add `pertag: HashMap::new(),`

Update `Monitor::new_with_values()`:
- Change signature from `pub fn new_with_values(mfact: f32, nmaster: i32, showbar: bool, topbar: bool)` to `pub fn new_with_values(showbar: bool, topbar: bool)`
- Remove `mfact,` and `nmaster,` from the struct init
- Add `pertag: HashMap::new(),`

Add this method inside `impl Monitor`:

```rust
/// Get or initialize state for the current tag mask.
pub fn pertag_state(&mut self) -> &mut PertagState {
    let mask = self.selected_tags().bits();
    self.pertag.entry(mask).or_insert(PertagState::default())
}
```

- [ ] **Step 1: Update Monitor struct**

Remove the two fields, add `pertag`.

- [ ] **Step 2: Update Monitor::default()**

Remove fields, add `pertag: HashMap::new()`.

- [ ] **Step 3: Update Monitor::new_with_values()**

Remove mfact/nmaster params and fields, add `pertag: HashMap::new()`.

- [ ] **Step 4: Add pertag_state() method**

- [ ] **Step 5: Verify build**

Run: `cargo build 2>&1 | head -50`
Expected: compilation errors in files that still use `mon.nmaster` / `mon.mfact` — expected.

- [ ] **Step 6: Commit**

```bash
git add src/types/monitor.rs
git commit -m "feat: add pertag HashMap to Monitor, remove mfact/nmaster fields"
```

---

## Task 3: Remove nmaster/mfact/showbar/layouts from `Tag` and update `init_tags`

**Files:**
- Modify: `src/types/tag.rs`
- Modify: `src/types/monitor.rs` (init_tags signature)
- Modify: `src/globals.rs` (tag_template type and build_tag_template)
- Modify: `src/monitor.rs` (all call sites of init_tags and new_with_values)

**A. `src/types/tag.rs`**

Remove from the `Tag` struct:
```rust
// REMOVE:
// pub nmaster: i32,
// pub mfact: f32,
// pub showbar: bool,
// pub layouts: TagLayouts,
```

Update `Tag::default()`:
```rust
impl Default for Tag {
    fn default() -> Self {
        Self {
            name: String::new(),
            alt_name: String::new(),
        }
    }
}
```

**B. `src/types/monitor.rs` — init_tags**

Change the signature:
```rust
pub fn init_tags(&mut self, template: &[TagNames]) {
    self.tags = template.to_vec();
}
```

**C. `src/globals.rs`**

Change `tag_template: Vec<crate::types::Tag>` to `Vec<crate::types::monitor::TagNames>`.

Change `build_tag_template`:
```rust
pub fn build_tag_template(cfg: &crate::config::Config) -> Vec<crate::types::monitor::TagNames> {
    let num_tags = cfg.num_tags;
    let mut template = Vec::with_capacity(num_tags);
    for i in 0..num_tags {
        let name = if i < cfg.tag_names.len() {
            cfg.tag_names[i].clone()
        } else {
            format!("{}", i + 1)
        };
        let alt_name = if i < cfg.tag_alt_names.len() {
            cfg.tag_alt_names[i].clone()
        } else {
            String::new()
        };
        template.push(TagNames { name, alt_name });
    }
    template
}
```

Update `apply_tags_config` where it calls `mon.init_tags(&template)` — the types now match directly.

**D. `src/monitor.rs` — new_with_values call sites**

Every call to `Monitor::new_with_values(...)` loses the first two arguments. Find and update each:
- Line ~460: `Monitor::new_with_values(ctx.core().globals().cfg.mfact, ctx.core().globals().cfg.nmaster, ctx.core().globals().cfg.show_bar, ctx.core().globals().cfg.top_bar)` → `Monitor::new_with_values(ctx.core().globals().cfg.show_bar, ctx.core().globals().cfg.top_bar)`
- Line ~586: similarly strip first two args
- Line ~698: similarly strip first two args

- [ ] **Step 1: Strip fields from Tag struct**

Remove nmaster, mfact, showbar, layouts.

- [ ] **Step 2: Strip fields from Tag::default()**

Remove the four fields.

- [ ] **Step 3: Change init_tags signature to TagNames**

- [ ] **Step 4: Update globals.rs tag_template type and build_tag_template**

- [ ] **Step 5: Update monitor.rs new_with_values call sites**

Remove first two args from each call.

- [ ] **Step 6: Verify build**

Run: `cargo build 2>&1 | head -60`
Expected: more errors from code still reading `tag.nmaster`, `tag.mfact`, `tag.layouts`, `tag.showbar`, `Monitor::nmaster`, `Monitor::mfact`.

- [ ] **Step 7: Commit**

```bash
git add src/types/tag.rs src/types/monitor.rs src/globals.rs src/monitor.rs
git commit -m "refactor: remove nmaster/mfact/showbar/layouts from Tag, init_tags takes TagNames"
```

---

## Task 4: Delete `apply_pertag_settings()` and fix bar toggle

**Files:**
- Modify: `src/tags/view.rs`
- Modify: `src/toggles.rs`

**A. `src/tags/view.rs`**

Delete the entire `apply_pertag_settings` function (~lines 332-348):
```rust
pub(super) fn apply_pertag_settings(core: &mut CoreCtx) {
    let (nmaster, mfact) = {
        let mon = core.globals().selected_monitor();
        let Some(current_tag) = mon.current_tag else {
            return;
        };
        if current_tag >= mon.tags.len() {
            return;
        }
        let tag = &mon.tags[current_tag - 1];
        (tag.nmaster, tag.mfact)
    };

    let mon = core.globals_mut().selected_monitor_mut();
    mon.nmaster = nmaster;
    mon.mfact = mfact;
}
```

Also look through `tags/view.rs` for any remaining direct reads of `tag.nmaster`, `tag.mfact`, `tag.showbar`, `tag.layouts` fields and remove them.

**B. `src/toggles.rs`**

The bar toggle currently does:
```rust
selmon.showbar = !selmon.showbar;
if let Some(current_tag) = selmon.current_tag
    && current_tag <= selmon.tags.len()
{
    selmon.tags[current_tag - 1].showbar = selmon.showbar;
}
```

Change to:
```rust
selmon.pertag_state().showbar = !selmon.pertag_state().showbar;
selmon.update_bar_position(bar_height);
```

Remove the `if let Some(current_tag)...` block entirely. The `bar_height` variable is already in scope above.

- [ ] **Step 1: Delete apply_pertag_settings from tags/view.rs**

- [ ] **Step 2: Fix bar toggle in toggles.rs**

Replace the sync block with `selmon.pertag_state().showbar = !selmon.pertag_state().showbar`.

- [ ] **Step 3: Verify build**

Run: `cargo build 2>&1 | head -60`
Expected: more errors from remaining direct field accesses.

- [ ] **Step 4: Commit**

```bash
git add src/tags/view.rs src/toggles.rs
git commit -m "refactor: delete apply_pertag_settings, use pertag_state for bar toggle"
```

---

## Task 5: Fix all remaining `mon.showbar` direct reads

**Files:**
- Modify: `src/floating/overlay.rs`
- Modify: `src/floating/movement.rs`
- Modify: `src/floating/snap.rs`
- Modify: `src/mouse/drag/move_drop.rs`
- Modify: `src/mouse/hover.rs`
- Modify: `src/layouts/algo/overview.rs`
- Modify: `src/systray/x11.rs`
- Modify: `src/client/rules.rs`
- Modify: `src/backend/x11/events/handlers.rs`
- Modify: `src/types/monitor.rs` (`shows_bar`, `update_bar_position`)
- Modify: `src/toggles.rs` (already done in Task 4)

Each call site passes `mon.selected_tags()` to get the current mask. Pattern:
```rust
// Replace:
let base_offset = if mon.showbar { bar_height } else { 0 };
// With:
let base_offset = if mon.pertag_state().showbar { bar_height } else { 0 };
```

The change is identical at every site — just replace `mon.showbar` with `mon.pertag_state().showbar`. `pertag_state()` takes `&mut self` so these sites may need adjustment if they only had `&mon`. In that case, the caller must provide the mask. For read-only accesses, consider adding a helper:

```rust
/// Returns showbar state for the given tag mask.
pub fn showbar_for_mask(&self, mask: TagMask) -> bool {
    self.pertag.get(&mask.bits()).map(|s| s.showbar).unwrap_or(true)
}
```

Then `mon.showbar` at read-only call sites becomes `mon.showbar_for_mask(mon.selected_tags())`.

For sites that already have `&mut mon` or are in a context that can get it, use `mon.pertag_state().showbar`.

Here is the breakdown per file:

**`src/floating/overlay.rs:38`** — `calculate_yoffset` has `&mon`. Add a `mask: TagMask` param and use `mon.pertag_state().showbar`:
```rust
fn calculate_yoffset(ctx: &WmCtx, mon: &mut Monitor, mask: TagMask) -> i32 {
    let bar_height = ctx.core().globals().cfg.bar_height;
    let base_offset = if mon.pertag_state().showbar { bar_height } else { 0 };
```
Update the call site in `place_overlay` to pass `mon.selected_tags()`.

**`src/floating/movement.rs:98`** — `snap_window` has `&mon`. Add `mask: TagMask` param:
```rust
fn snap_window(ctx: &mut WmCtx, win: WindowId, mon: &mut Monitor, mask: TagMask) {
    let showbar = mon.pertag_state().showbar;
```
Update call site.

**`src/floating/snap.rs:197`** — `snap_monitor` has `&m`. Add `mask: TagMask` param:
```rust
fn snap_monitor(ctx: &mut WmCtx, monitor_id: MonitorId, mask: TagMask) {
    let mony = m.monitor_rect.y
        + if m.pertag_state().showbar {
```
Update call site.

**`src/mouse/drag/move_drop.rs:72,81`** — add `mask: TagMask` param:
```rust
fn adjust_drop_y(ctx: &mut WmCtx, win: WindowId, mon: &mut Monitor, mask: TagMask, ...) {
    if y <= mon.monitor_rect.y + if mon.pertag_state().showbar { mon.bar_height } else { 5 } {
```

**`src/mouse/hover.rs:81,130,203`** — add `mask: TagMask` param:
```rust
fn hover_on_monitor(ctx: &mut WmCtx, mon: &mut Monitor, mask: TagMask, ...) {
    if mon.pertag_state().showbar && y < mon.monitor_rect.y + mon.bar_height {
```

**`src/layouts/algo/overview.rs:56`** — already takes `&mon`. Add `mask: TagMask` param:
```rust
pub fn overview(ctx: &mut WmCtx, mon: &mut Monitor, mask: TagMask) {
    let base_offset = if mon.pertag_state().showbar { bar_height } else { 0 };
```
Update call sites.

**`src/systray/x11.rs:212`** — add `mask: TagMask` param.

**`src/client/rules.rs:84`** — `m.showbar` in match arm. Add helper or pass mask.

**`src/backend/x11/events/handlers.rs:421`** — `selmon.showbar`. Change to `selmon.pertag_state().showbar` (caller has `&mut selmon`).

**`src/types/monitor.rs:358` (`shows_bar`)** — `self.current_tag().map(|t| t.showbar).unwrap_or(true)` → `self.pertag_state().showbar`

**`src/types/monitor.rs:420` (`update_bar_position`)** — `if self.showbar` → `if self.pertag_state().showbar`

- [ ] **Step 1: Update floating/overlay.rs calculate_yoffset**

Add mask param, use pertag_state.

- [ ] **Step 2: Update floating/movement.rs snap_window**

Add mask param, use pertag_state.

- [ ] **Step 3: Update floating/snap.rs snap_monitor**

Add mask param, use pertag_state.

- [ ] **Step 4: Update mouse/drag/move_drop.rs**

Add mask param to relevant functions, use pertag_state.

- [ ] **Step 5: Update mouse/hover.rs**

Add mask param to relevant functions, use pertag_state.

- [ ] **Step 6: Update layouts/algo/overview.rs**

Add mask param, use pertag_state.

- [ ] **Step 7: Update systray/x11.rs**

Add mask param, use pertag_state.

- [ ] **Step 8: Update client/rules.rs**

Use showbar_for_mask or pass mask.

- [ ] **Step 9: Update backend/x11/events/handlers.rs**

Use pertag_state (already has &mut).

- [ ] **Step 10: Update Monitor::shows_bar and update_bar_position**

Use pertag_state directly.

- [ ] **Step 11: Verify build**

Run: `cargo build 2>&1 | head -80`
Expected: errors from layout algos writing nmaster/mfact, and from layouts/manager.rs. Fix in Task 6.

- [ ] **Step 12: Commit**

```bash
git add src/floating/overlay.rs src/floating/movement.rs src/floating/snap.rs
git add src/mouse/drag/move_drop.rs src/mouse/hover.rs
git add src/layouts/algo/overview.rs src/systray/x11.rs
git add src/client/rules.rs src/backend/x11/events/handlers.rs
git add src/types/monitor.rs
git commit -m "refactor: all mon.showbar reads go through pertag_state"
```

---

## Task 6: Fix layout algorithms writing `monitor.nmaster` directly

**Files:**
- Modify: `src/layouts/algo/tile.rs`
- Modify: `src/layouts/algo/stack.rs`
- Modify: `src/layouts/algo/three_column.rs`
- Modify: `src/layouts/manager.rs` (sync after layout)

The layout algos (`tile.rs`, `stack.rs`, `three_column.rs`) currently write to `monitor.nmaster` and `monitor.mfact` directly. After our changes, `monitor.nmaster` field no longer exists. The layout algos should NOT write to the pertag map directly — instead, they should read/write local variables, and the caller (`run_layout` / `arrange_monitor`) syncs the values to pertag state after the layout completes.

The pattern: in each layout function, replace `monitor.nmaster = ...` with a local variable `nmaster`, and replace `monitor.mfact = ...` with a local `mfact`. At the end of `arrange_monitor` (in `layouts/manager.rs`), sync `nmaster`/`mfact` back to `mon.pertag_state()`.

**`src/layouts/algo/tile.rs`**

Replace all `monitor.nmaster = ...` with `let mut nmaster = ...` and `monitor.mfact = ...` with `let mfact = ...`.

Specifically:
- Line 59: `monitor.nmaster = tiled_client_count as i32;` → local variable, then recursive call passes it
- Lines 144-145: `monitor.mfact = 0.7; monitor.nmaster = 1;` → local variables
- Lines 154-155: `monitor.mfact = 0.2; monitor.nmaster = 2;` → local variables

The function signature stays `pub fn tile(ctx: &mut WmCtx<'_>, monitor: &mut Monitor)` — no change needed.

**`src/layouts/algo/stack.rs`**

Replace all `monitor.nmaster` reads with a local `nmaster` variable initialized from `monitor.pertag_state().nmaster`. Similarly for `monitor.mfact`.

**`src/layouts/algo/three_column.rs`**

Replace `monitor.mfact` read with local `mfact` from `monitor.pertag_state().mfact`.

**`src/layouts/manager.rs`**

After calling `run_layout(ctx, monitor_id)` in `arrange_monitor`, sync the layout state back to pertag:

```rust
fn arrange_monitor(ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
    // ... existing code ...

    // Sync nmaster/mfact/layouts from pertag state to monitor for layout algos
    {
        let mon = ctx.core_mut().globals_mut().monitor_mut(monitor_id).unwrap();
        let pertag = mon.pertag_state();
        mon.nmaster = pertag.nmaster;
        mon.mfact = pertag.mfact;
    }

    run_layout(ctx, monitor_id);

    // After layout, sync back
    {
        let mon = ctx.core_mut().globals_mut().monitor_mut(monitor_id).unwrap();
        let pertag = mon.pertag_state();
        pertag.nmaster = mon.nmaster;
        pertag.mfact = mon.mfact;
        // layouts are handled by set_layout/toggle_layout already
    }
    // ...
}
```

Actually, this is getting complicated because layout algos need to READ nmaster/mfact during execution. The cleanest solution: before calling `run_layout`, copy from pertag to local shadow fields on Monitor (`nmaster`/`mfact`), let the algos read/write those, then after `run_layout` copy back to pertag. But we removed those fields.

Better approach: layout algos read from `monitor.pertag_state().nmaster` at the start, use local variables, and write back to `monitor.pertag_state()` at the end. Since `run_layout` calls the specific layout functions and we control that code, we can add the sync in `run_layout`.

Wait, actually the simplest approach: keep `nmaster` and `mfact` as fields on `Monitor` for the layout algos to read/write, but make them "scratch" fields that are populated from pertag before layout and written back after. But that defeats the purpose of removing them.

Alternative: layout algos take the nmaster/mfact values as parameters or return them. Let me think...

Actually, the cleanest approach is: layout algos receive `&mut Monitor` as before, but read nmaster/mfact from `monitor.pertag_state()`, and write back to `monitor.pertag_state()` at the END of each layout function (before returning). The layout algos can call a helper at the end:

```rust
fn sync_from_pertag(monitor: &mut Monitor) {
    let p = monitor.pertag_state();
    let nmaster = p.nmaster;
    let mfact = p.mfact;
    // these are readable for the duration of the layout
}
```

No, this is getting circular. Let me reconsider.

The layout algos need to READ nmaster/mfact at the start and WRITE at the end (tile.rs does `monitor.nmaster = tiled_client_count as i32` to clamp). With our new design:

Option A: Layout algos read `monitor.pertag_state().nmaster` and write `monitor.pertag_state().nmaster`. Since they're called via `&mut Monitor`, they can call `monitor.pertag_state()`.

Option B: Sync to/from temporary scratch fields on Monitor. But we removed those fields to reduce confusion. Adding them back as "scratch" defeats the purpose.

Option C: `run_layout` does the sync dance — copies from pertag to local variables, passes them to the layout algo via a wrapper or helper, then copies back. But layout algos use `monitor.nmaster` directly in the function body, not as parameters.

Looking at `tile.rs` line 59: `monitor.nmaster = tiled_client_count as i32;` — this writes to monitor, and the recursive call `tile(ctx, monitor)` expects the updated value. So nmaster needs to be readable AND writable through monitor.

The cleanest path: keep `nmaster` and `mfact` as fields on `Monitor` but make them the "current active" values (synced from pertag before layout, synced back after). Layout algos read/write them normally. After layout, `arrange_monitor` syncs back to pertag. This is basically the current design but with explicit sync points.

Wait, but we already removed those fields from Monitor. Let me re-add them as "scratch" fields used by layout algos:

In `Monitor`:
```rust
/// Current nmaster value used by layout algos. Sync from pertag_state() before layout.
pub nmaster: i32,
/// Current mfact value used by layout algos. Sync from pertag_state() before layout.
pub mfact: f32,
```

Then in `arrange_monitor`:
1. Before `run_layout`: `mon.nmaster = mon.pertag_state().nmaster; mon.mfact = mon.pertag_state().mfact;`
2. After `run_layout`: `mon.pertag_state().nmaster = mon.nmaster; mon.pertag_state().mfact = mon.mfact;`

This way layout algos don't need to change at all (they still read/write `monitor.nmaster/mfact`), and the sync happens at the arrange level. Let me use this approach — it's the least invasive for the layout algos.

So revise the plan: re-add `nmaster` and `mfact` as scratch fields on Monitor, used by layout algos, with explicit sync from/to pertag in `arrange_monitor`.

Let me update Task 2 and Task 6 accordingly.

In Task 2, keep `nmaster` and `mfact` on Monitor (as scratch fields for layout algos).

In Task 6, in `arrange_monitor`:
- Before `run_layout`: sync from pertag to Monitor fields
- After `run_layout`: sync from Monitor fields back to pertag

And the layout algos can stay as-is (still reading `monitor.nmaster`).

Wait, but there's a problem: in `tile.rs` line 59, there's a recursive call `tile(ctx, monitor)` after writing `monitor.nmaster = tiled_client_count`. The recursive call re-reads `monitor.nmaster`, which is fine since it's a scratch field. After the recursive call returns, what? The original `monitor.nmaster` is whatever was set by the inner call. When tiled_client_count < nmaster, we set nmaster to client_count and recurse. After recursion, nmaster has the clamped value. The caller doesn't seem to need the value back — it's just used for the layout computation.

Actually looking more carefully at `tile.rs` lines 58-61:
```rust
if tiled_client_count > 1 && tiled_client_count < monitor.nmaster as u32 {
    monitor.nmaster = tiled_client_count as i32;
    tile(ctx, monitor);
    return;
}
```

This adjusts nmaster downward when there are few clients, then recurses. After return, the local nmaster is whatever was set inside. Since nmaster is a Monitor scratch field, it persists. Back in the caller (`run_layout` or `arrange_monitor`), we sync it back to pertag.

OK so the approach is:
1. In Task 2: keep `nmaster` and `mfact` on Monitor (scratch fields for layout algos), add `pertag` HashMap
2. In arrange_monitor: before run_layout, copy pertag_state → Monitor fields; after run_layout, copy Monitor fields → pertag_state

Let me rewrite Task 2 and Task 6 accordingly.

Actually, let me reconsider. The user explicitly said "nmaster and mfact should be stored per tag mask". The question is whether they should LIVE on Monitor (as scratch) or only in pertag. The "one source of truth" suggests they should ONLY be in pertag, and Monitor fields are just scratch for the layout algorithm to use. That's fine — the source of truth is pertag, the Monitor fields are a cache.

So keep the Monitor.nmaster/mfact fields as scratch/cache, sync before and after layout. This is clean.

But wait — `inc_nmaster_by` and `set_mfact` in layouts/manager.rs currently write BOTH to `mon.nmaster` and `mon.tags[tag-1].nmaster`. After our change, they should write to `mon.pertag_state().nmaster` (the source of truth) AND to `mon.nmaster` (the scratch). And then `arrange` syncs back to pertag.

Actually, that's redundant. If `inc_nmaster_by` writes to pertag, and arrange syncs pertag → mon.nmaster before layout, and after layout syncs mon.nmaster → pertag... that creates a round-trip. The simpler flow:
- `inc_nmaster_by` writes to `pertag_state().nmaster` (source of truth) AND to `mon.nmaster` (scratch cache)
- `arrange_monitor` doesn't need to sync — the scratch is already updated
- After layout, `arrange_monitor` syncs `mon.nmaster` → `pertag_state().nmaster` (in case layout algo modified it)

Hmm, but if layout algo modified `mon.nmaster` (like the tile.rs recursive clamp), we want that new value in pertag too. So yes, sync back after.

OK final approach:
- Layout algos read/write `monitor.nmaster`/`monitor.mfact` (scratch fields, they exist as before)
- `inc_nmaster_by` and `set_mfact` write to BOTH `pertag_state().nmaster` and `mon.nmaster`
- `arrange_monitor` BEFORE `run_layout`: copy `pertag_state().nmaster/mfact` → `mon.nmaster/mfact`
- `arrange_monitor` AFTER `run_layout`: copy `mon.nmaster/mfact` → `pertag_state().nmaster/mfact`

This means the layout algos don't need to change AT ALL. The only changes are in `arrange_monitor` (sync before/after) and in `inc_nmaster_by`/`set_mfact` (also write to pertag).

But we already removed `nmaster` and `mfact` from Monitor in Task 2. So in Task 2 we need to KEEP those fields as scratch fields. Let me update the plan.

In Task 2, we keep `nmaster` and `mfact` on Monitor as scratch fields, but ALSO add the `pertag` HashMap. The scratch fields are populated from pertag at arrange time and synced back after.

So in Task 2: don't remove `nmaster` and `mfact` from Monitor — keep them as scratch.

Let me revise the plan now.

---

- [ ] **Step 1: Update tile.rs — use local nmaster, write back to monitor**

In `tile.rs`, replace the direct `monitor.nmaster` writes with local variables. The key change is line 59 (`monitor.nmaster = tiled_client_count as i32`). Change to a local `nmaster` that shadows the field, then write back at function end:

```rust
let mut nmaster = monitor.pertag_state().nmaster;
// ... use nmaster instead of monitor.nmaster ...
monitor.pertag_state().nmaster = nmaster;
```

Lines 144-145, 154-155 also write to `monitor.mfact`/`monitor.nmaster` — use local variables and write back.

- [ ] **Step 2: Update stack.rs — read nmaster/mfact from pertag_state**

Read `nmaster` from `monitor.pertag_state().nmaster` at function start, use local variable, write back at end.

- [ ] **Step 3: Update three_column.rs — read mfact from pertag_state**

Read `mfact` from `monitor.pertag_state().mfact` at function start, use local, write back at end.

- [ ] **Step 4: Update arrange_monitor in layouts/manager.rs**

Before `run_layout`:
```rust
{
    let mon = ctx.core_mut().globals_mut().monitor_mut(monitor_id).unwrap();
    let pertag = mon.pertag_state();
    mon.nmaster = pertag.nmaster;
    mon.mfact = pertag.mfact;
}
```

After `run_layout`:
```rust
{
    let mon = ctx.core_mut().globals_mut().monitor_mut(monitor_id).unwrap();
    let pertag = mon.pertag_state();
    pertag.nmaster = mon.nmaster;
    pertag.mfact = mon.mfact;
}
```

Note: Need to re-add `nmaster` and `mfact` as fields on Monitor (they were removed in Task 2). Actually, we kept them as scratch in Task 2, so they're still there. Good.

- [ ] **Step 5: Verify build**

Run: `cargo build 2>&1 | head -80`
Expected: errors in `inc_nmaster_by` and `set_mfact` still writing to old fields.

- [ ] **Step 6: Commit**

```bash
git add src/layouts/algo/tile.rs src/layouts/algo/stack.rs src/layouts/algo/three_column.rs
git add src/layouts/manager.rs
git commit -m "refactor: layout algos read/write nmaster/mfact via pertag_state scratch fields"
```

---

## Task 7: Fix `inc_nmaster_by` and `set_mfact` in layouts/manager.rs

**Files:**
- Modify: `src/layouts/manager.rs`

These functions currently write to `m.nmaster` and `m.tags[tag-1].nmaster` / `m.mfact` and `m.tags[tag-1].mfact`. Change to write to both the scratch fields AND pertag_state:

**`inc_nmaster_by`** (lines 310-330):
```rust
let m = ctx.core_mut().globals_mut().selected_monitor_mut();
if delta > 0 && m.nmaster >= ccount {
    m.nmaster = ccount;
} else {
    let new_nmaster = max(m.nmaster + delta, 0);
    m.nmaster = new_nmaster;
    // REMOVE the tags write entirely — nmaster is now only in pertag and scratch
}
```
After the nmaster logic, sync to pertag:
```rust
m.pertag_state().nmaster = m.nmaster;
```

**`set_mfact`** (lines 332-380):
```rust
let m = ctx.core_mut().globals_mut().selected_monitor_mut();
m.mfact = new_mfact;
// REMOVE the tags write — mfact is now only in pertag and scratch
m.pertag_state().mfact = m.mfact;
```

Also remove the now-unused `use crate::types::tag::Tag;` import if it was added only for the `m.tags[tag-1]` access.

- [ ] **Step 1: Fix inc_nmaster_by**

Remove the `m.tags[tag-1].nmaster = new_nmaster;` line. Add `m.pertag_state().nmaster = m.nmaster;`.

- [ ] **Step 2: Fix set_mfact**

Remove the `m.tags[tag-1].mfact = new_mfact;` line. Add `m.pertag_state().mfact = m.mfact;`.

- [ ] **Step 3: Verify build**

Run: `cargo build 2>&1 | head -60`
Expected: should compile successfully.

- [ ] **Step 4: Commit**

```bash
git add src/layouts/manager.rs
git commit -m "refactor: inc_nmaster_by and set_mfact write to pertag_state"
```

---

## Task 8: Fix bar/paint.rs and bar/scene.rs for layout_symbol

**Files:**
- Modify: `src/bar/paint.rs`
- Modify: `src/bar/scene.rs`

These currently call `mon.layout_symbol()` which reads `current_tag().map(|t| t.layouts.symbol())`. Update `Monitor::layout_symbol()` to use `pertag_state()`:

In `src/types/monitor.rs`:

```rust
pub fn layout_symbol(&self) -> String {
    self.pertag_state().layouts.symbol().to_string()
}
```

Note: `pertag_state()` requires `&mut self`. `layout_symbol()` is called in `bar/scene.rs:219` with `&mon`. Need to check if we can get a mutable reference there, or if we need a `&self` version.

`bar/scene.rs:219`:
```rust
layout_symbol: mon.layout_symbol(),
```

`mon` is `&Monitor` from `ctx.core().globals().monitor(...)`. We need `&mut mon` to call `pertag_state()`. The simplest fix: change to `mon.pertag_state().layouts.symbol().to_string()` at the call site, since we control this code:

```rust
layout_symbol: ctx.core().globals().monitor(id).map(|m| {
    m.pertag_state().layouts.symbol().to_string()
}).unwrap_or_else(|| "[]=".to_string()),
```

Or better: add `layout_symbol_for_mask(&self, mask: TagMask)` that takes the mask and uses `self.pertag.get(&mask.bits())`.

Actually, the easiest fix: add an immutable helper:
```rust
pub fn layout_symbol_for_mask(&self, mask: TagMask) -> String {
    self.pertag.get(&mask.bits()).map(|s| s.layouts.symbol().to_string()).unwrap_or_else(|| "[]=".to_string())
}
```

Then `bar/scene.rs` and `bar/widgets.rs` can call it with `selected_tags()`.

- [ ] **Step 1: Add layout_symbol_for_mask to Monitor**

Add the helper method using `&self` (immutable) to look up layouts by mask.

- [ ] **Step 2: Update bar/scene.rs call site**

Use `layout_symbol_for_mask(mon.selected_tags())`.

- [ ] **Step 3: Update bar/widgets.rs call site**

Use `layout_symbol_for_mask(m.selected_tags())`.

- [ ] **Step 4: Verify build**

Run: `cargo build 2>&1 | head -40`

- [ ] **Step 5: Commit**

```bash
git add src/bar/scene.rs src/bar/widgets.rs src/types/monitor.rs
git commit -m "refactor: bar layout_symbol uses pertag_state lookup by mask"
```

---

## Final Verification

After all tasks:
1. Run `cargo build` — must compile cleanly
2. Run `cargo test` — must pass
3. Review that `Monitor::nmaster` and `Monitor::mfact` are ONLY written in `arrange_monitor` (sync back) and in `inc_nmaster_by`/`set_mfact` (dual-write to scratch + pertag)
4. Verify no direct reads of `tag.nmaster`, `tag.mfact`, `tag.showbar`, `tag.layouts` remain
5. Verify `apply_pertag_settings` is deleted

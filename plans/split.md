# Completing the Window-Manager State Split

## Status

This is the follow-up plan after commits `8654a2c2` and `18e58fbc`.
Those commits made the ownership split real, but the dependency split is still
in progress. This document describes only the remaining work; the broader
design rationale remains in `plans/rearch.md`.

Breaking internal APIs are allowed. User-visible behavior, IPC, configuration,
and backend behavior should remain stable unless changed in a separate commit.

## What is already improved

- `Globals` has been replaced by an explicit `WmModel` plus runtime state.
- `WmModel` owns the related client, monitor, and tag graph.
- `PendingWork` remains separately owned by `Wm`.
- Configuration replacement remains atomic and preserves backend-derived state.
- Visibility, window-mode, and view-selection policy accept narrow model
  dependencies and have direct backend-free tests.
- `window_protocol` now belongs to `WindowOps` rather than `OutputOps`.
- Several free-standing value operations moved onto their natural types, such
  as rule matching, snap navigation, client mode queries, and monitor geometry
  queries.

These changes are architectural improvements, not just renames. In particular,
pure policy can now be tested without constructing X11 or Wayland state.

## Remaining problems

### `CoreState` is still a broad state gateway

`CoreState` groups model, configuration, behavior, drag state, keyboard state,
and pending launches. `CoreCtx::state()` and `state_mut()` expose all of it.
This is useful as a migration adapter, but it has essentially the same reach as
the old `Globals` API if it remains available to ordinary feature code.

At the time this plan was written there were approximately:

- 105 `state()` calls;
- 49 `state_mut()` calls;
- 395 `model()` calls;
- 139 `model_mut()` calls;
- 280 functions taking `&mut WmCtx`.

The counts are not targets by themselves. They identify where dependency
narrowing is incomplete.

### `CoreCtx` is accumulating policy

Color-scheme selection and mode normalization currently live on `CoreCtx`.
Those methods combine presentation policy with model/config reads and make the
context more than an orchestration adapter. If this continues, `CoreCtx` will
become the new god object even though ownership was split successfully.

Presentation calculations belong in the bar/presentation domain and should
accept explicit inputs. Mode normalization belongs with behavior/config reload
policy. `CoreCtx` should retain only orchestration concerns such as scheduling,
quit state, backend-neutral effect helpers, and temporary reborrowing.

### Model invariants can still be bypassed

`WmModel.clients`, `monitors`, and `tags` are crate-visible, so any module can
mutate one side of the graph without maintaining the other. Access to read-only
manager queries is reasonable during migration, but topology changes should be
owned by `WmModel` operations.

The highest-risk invariants are:

- client monitor assignment versus monitor client lists;
- focus lists and persistent z-order;
- selected clients after removal or transfer;
- monitor replacement/remapping;
- tag masks and per-monitor tag state.

### Boundary and policy code are still mixed

Many feature operations still take `&mut WmCtx` and perform model mutation,
focus changes, backend effects, arranging, and flushing in one function. A
`WmCtx` parameter is appropriate at event/action/IPC boundaries, but internal
decisions should take managers or value inputs and return plans/outcomes.

## Target shape

`Wm` remains the composition root:

```rust
pub struct Wm {
    model: WmModel,
    config: RuntimeConfig,
    behavior: WmBehavior,
    interaction: InteractionState,
    keyboard: KeyboardLayoutState,
    launches: PendingLaunchTracker,
    work: PendingWork,
    bar: BarState,
    focus: FocusState,
    running: bool,
    backend: Backend,
}
```

It is acceptable to keep `CoreState` temporarily to make construction and
reborrowing manageable. The required end state is that feature code cannot ask
for unrestricted `&mut CoreState`. Flattening its fields into `Wm` is preferable
once broad access has been removed, but encapsulation matters more than the
physical nesting.

`CoreCtx` should be an orchestration adapter, not a policy owner:

```rust
pub struct CoreCtx<'a> {
    model: &'a mut WmModel,
    config: &'a mut RuntimeConfig,
    behavior: &'a mut WmBehavior,
    interaction: &'a mut InteractionState,
    keyboard: &'a mut KeyboardLayoutState,
    launches: &'a mut PendingLaunchTracker,
    work: &'a mut PendingWork,
    bar: &'a mut BarState,
    focus: &'a mut FocusState,
    running: &'a mut bool,
}
```

Not every operation should receive this type. Boundary code may use it to
coordinate narrow operations; internal policy should receive only what it
actually uses.

## Implementation plan

### Phase 1: Finish regression coverage for model seams

Complete the protection needed before tightening visibility.

1. Add direct tests for client transfer. Cover:
   - missing clients and invalid target monitors;
   - source removal and target insertion;
   - monitor assignment, tags, and sticky reset;
   - focus-list and z-order duplicate prevention;
   - scratchpad behavior;
   - floating versus tiled arrange outcomes.
2. Extend view-selection tests to cover selected-monitor isolation and tag-set
   buffer switching.
3. Add table-driven tests for `SnapPosition::next` and `Rule::matches`, since
   these policies were moved onto value types.
4. Test monitor bar/fullscreen predicates with inactive, hidden, sticky, and
   true-fullscreen clients.

Keep these tests backend-free.

### Phase 2: Put cross-manager invariants on `WmModel`

Move topology-changing operations onto `WmModel` before making its fields
private.

Priority operations:

1. `transfer_client`;
2. attach/detach and z-order maintenance;
3. managed-client insertion/removal;
4. monitor replacement/remapping;
5. reorder and selected-client repair;
6. tag changes that update more than one model object.

Each method should either complete the full invariant-preserving transition or
return a small plan/outcome describing required effects. Do not expose a pair
of public operations that callers must invoke in the right order.

Example:

```rust
let outcome = model.transfer_client(win, target)?;
apply_transfer_effects(ctx, outcome);
```

After callers migrate, make manager fields private. Add focused read access
only where a caller genuinely needs manager queries.

### Phase 3: Remove presentation policy from `CoreCtx`

Move these methods out of `contexts.rs`:

- status, tag, window, and close-button scheme selection;
- tag hover fill selection;
- current-mode normalization.

Create pure presentation functions in the bar domain. They should take
`&WmModel`, the relevant color configuration, and explicit values. If repeated
inputs form a stable concept, introduce a read-only `BarPresentationInput` or
snapshot; do not pass `CoreCtx` to replace a shorter argument list.

Move mode normalization to behavior/config policy, for example:

```rust
behavior.normalize_mode(&config.bindings.modes);
```

Completion condition: `contexts.rs` contains orchestration and backend context
plumbing, not UI or window-management policy.

### Phase 4: Eliminate `state_mut()` subsystem by subsystem

Migrate mutable broad access first because it permits accidental coupling.

Recommended order:

1. tags and view;
2. floating state and scratchpads;
3. layouts and visibility;
4. monitor management;
5. focus;
6. mouse/drag/resize;
7. keyboard and input;
8. backend lifecycle callbacks;
9. IPC, reload, and startup.

For each subsystem:

1. identify the exact model/runtime fields used;
2. extract pure decisions into narrow functions;
3. return owned plans so model borrows end before backend effects;
4. keep effect ordering explicit in the boundary wrapper;
5. replace `state_mut()`;
6. run focused tests before moving to the next subsystem.

Remove `CoreCtx::state_mut()` when the last use is gone. Do not retain it as a
convenience escape hatch.

### Phase 5: Eliminate broad immutable state access

Replace `state()` with explicit category reads or narrow function arguments.
Immutable broad access is less dangerous than mutable access, but it still
hides dependencies and encourages functions to grow silently.

Backend helpers should receive IDs, configuration slices, manager references,
or snapshots rather than `CoreState`. X11 property/bar helpers are a useful
first target because their actual inputs are usually small and stable.

Remove `CoreCtx::state()` when complete.

### Phase 6: Narrow internal `WmCtx` users

Classify every remaining `&mut WmCtx` function as one of:

- boundary/orchestrator: keep the context;
- model policy: replace with direct model dependencies;
- presentation policy: replace with model/config snapshots;
- backend effect application: use the relevant backend-specific context or
  capability plus a plan;
- mixed operation: split into decision and application functions.

Start with high-churn paths relevant to future tiled dragging:

1. monitor transfer and client reorder;
2. layout changes and placement;
3. drag/drop target calculation;
4. focus resolution;
5. tag movement and swapping.

These should expose reusable model operations such as `move_before`,
`move_after`, `swap_clients`, or `transfer_client`, with backend/layout work
queued by the orchestration layer. That will make future mouse-driven tiled
reordering a new input path over existing operations rather than a second
implementation of layout mutation.

### Phase 7: Give remaining runtime domains real owners

Replace raw state containers with focused types where they have behavior:

- `DragState` becomes or lives within `InteractionState`;
- `VecDeque<PendingLaunch>` becomes `PendingLaunchTracker` with record, match,
  consume, and prune methods;
- keyboard layout mutation remains on `KeyboardLayoutState` and validates
  indexes internally;
- behavior toggles and mode transitions remain on `WmBehavior`.

Avoid trivial getter/setter methods that merely rename public field access.
Methods should encode validation, transitions, or invariants.

### Phase 8: Flatten ownership and delete compatibility APIs

Once `state()` and `state_mut()` are gone:

1. move `CoreState` fields directly into `Wm`, or make `CoreState` a private
   implementation detail borrowed into disjoint `CoreCtx` fields;
2. remove delegating methods that only preserve old `Globals` call shapes;
3. make `WmModel` manager fields private;
4. remove duplicate normalization/helper implementations;
5. audit public and `pub(crate)` surface for obsolete migration APIs;
6. update module documentation to describe the final ownership graph.

## Commit strategy

Keep commits reviewable and behavior-preserving:

1. tests for one policy/invariant;
2. narrow that policy's signature;
3. move ownership or restrict visibility;
4. migrate callers;
5. remove the compatibility API.

Avoid another repository-wide mechanical commit if a subsystem can be completed
end to end. Small vertical slices provide better compiler guidance and make
backend-effect ordering easier to review.

## Validation

For every phase:

- `cargo fmt --check`;
- `cargo check --all-targets --all-features`;
- `cargo test --all-targets --all-features`;
- `git diff --check`;
- focused unit tests for the changed policy or invariant.

Do not use release builds.

Manual smoke testing should cover both backends when practical:

- startup and reload;
- focus and tag switching;
- floating/tiling/maximize transitions;
- client transfer and monitor hotplug/reorder;
- visibility and fullscreen bar behavior;
- mouse move/resize and tag dragging;
- layout changes and persistent z-order.

## Completion criteria

The split is complete when:

1. `WmModel` privately owns the client/monitor/tag graph and its cross-manager
   invariant-changing operations.
2. Feature code cannot obtain unrestricted `&CoreState` or `&mut CoreState`.
3. `CoreCtx` contains orchestration helpers, not model or presentation policy.
4. Internal policy functions declare narrow dependencies and are directly
   testable without a backend.
5. `WmCtx` is used primarily at event, action, IPC, and backend boundaries.
6. Configuration, behavior, interaction, keyboard, launches, pending work,
   bar, focus, and backend state have explicit ownership and lifecycles.
7. X11 and Wayland effect application remains separate where their semantics
   differ.
8. Transfer, removal, monitor remapping, view changes, visibility, and window
   mode transitions have direct invariant-focused tests.
9. No compatibility accessor recreates the old god-object dependency path.


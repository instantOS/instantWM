# Window Manager State and Dependency Rear-Architecture

## Status

Proposed architecture. Breaking internal API changes are allowed. Runtime behavior should remain unchanged unless a later, separately reviewed change explicitly alters policy.

## Goal

The goal is to make each window-manager operation declare the state it reads, the state it mutates, and the external effects it performs.

The current architecture has already made useful progress: runtime configuration is grouped, pending work is owned separately by `Wm`, interaction state is grouped, and backend capabilities have begun to split into window, pointer, and output operations. The remaining problem is that `Globals`, `CoreCtx`, and `WmCtx` still act as broad dependency gateways. A function receiving `&mut WmCtx` can reach nearly every part of the window manager, even when it only needs clients and monitors or one backend effect.

This obscures dependencies, increases borrow pressure, makes model behavior difficult to test without backend setup, and allows unrelated subsystems to become coupled accidentally.

The intended result is not to eliminate the top-level owner. `Wm` should continue to own the complete running window manager. The change is to stop leaking that complete ownership graph into internal operations.

## Design Principles

1. **Keep one composition root.** `Wm` owns runtime state and the selected backend.
2. **Separate authoritative model state from runtime services and transient state.** Clients, monitors, and tags form the core model; configuration, input interactions, work queues, rendering caches, and backend state have different lifecycles.
3. **Use narrow function signatures.** Internal functions receive the smallest truthful set of references they need.
4. **Keep broad contexts at boundaries.** Event handlers, action dispatch, IPC entry points, and backend callbacks may receive `WmCtx`; their internal model operations should not.
5. **Separate decisions from effects.** Model functions produce state changes, plans, or outcomes. Backend-specific code applies X11 or Wayland effects after model borrows end.
6. **Do not abstract away real backend differences.** X11 and Wayland may share policy while retaining different application mechanics.
7. **Preserve cross-object invariants in one place.** Operations that atomically maintain clients and monitors belong on the aggregate model rather than being split to satisfy an arbitrary size target.
8. **Prefer direct references over state-access traits.** Traits are for external capabilities, not for disguising a service locator.

## Current Problem

`Globals` currently owns several unrelated domains:

- runtime configuration;
- clients, monitors, and tags;
- behavior toggles and current mode;
- drag, gesture, and hover interaction state;
- keyboard layout runtime state;
- pending process launches.

`CoreCtx` then exposes unrestricted immutable and mutable access through `globals()` and `globals_mut()`. `WmCtx` adds backend access on top. As a result, signatures such as:

```rust
fn operation(ctx: &mut WmCtx)
```

do not communicate whether the operation modifies client topology, changes a tag selection, queues layout work, updates focus state, or performs backend I/O.

Simply renaming `Globals`, moving its fields into `Wm`, or adding more accessors would not solve this. If every caller can still access every field through one context, the dependency structure remains unchanged.

## Target Ownership

```rust
pub struct Wm {
    // Authoritative backend-neutral window-manager state.
    model: WmModel,

    // Reloadable policy and presentation configuration.
    config: RuntimeConfig,

    // Independent runtime domains.
    behavior: WmBehavior,
    interaction: InteractionState,
    keyboard: KeyboardLayoutState,
    launches: PendingLaunchTracker,

    // Orchestration state and caches.
    work: PendingWork,
    bar: BarState,
    focus: FocusState,
    running: bool,

    // Window-system composition boundary.
    backend: Backend,
}

pub struct WmModel {
    clients: ClientManager,
    monitors: MonitorManager,
    tags: TagSet,
}

pub struct InteractionState {
    drag: DragState,
    // Additional pointer/gesture modes belong here when they share the same
    // event-loop lifecycle. Configuration toggles do not.
}

pub struct PendingLaunchTracker {
    pending: VecDeque<PendingLaunch>,
}
```

The exact visibility of fields should be tightened during migration. The important boundary is ownership and operation signatures, not whether the first intermediate version uses fields or accessors.

### Why clients and monitors remain together

Clients and monitors are a cross-referenced graph:

- each client records its monitor;
- each monitor records client focus order;
- each monitor owns persistent z-order;
- selection and focus history refer to managed windows;
- monitor removal and reordering can require client ID remapping.

Operations such as transfer, attach, detach, monitor removal, and z-order repair must preserve these relationships atomically. Keeping them in `WmModel` gives those invariants one owner. Splitting them into unrelated top-level services would make consistency harder, not easier.

### Why configuration is not part of the model

`RuntimeConfig` describes reloadable policy and presentation settings. It is read by model calculations but is not part of the managed-window graph. Owning it beside `WmModel` makes configuration dependencies explicit and allows operations to take only the relevant configuration slice.

Configuration is not assumed to be immutable. IPC changes and backend-derived display or bar metrics currently mutate it. Those concerns may later be separated, but this architecture does not require that additional change.

### Why interaction, keyboard, and launches are separate

These domains have independent mutation patterns and lifecycles:

- interaction state changes during pointer and gesture event sequences;
- keyboard layout state changes through input configuration and layout switching;
- pending launches form a time-bounded queue consumed when windows are managed.

None of them participates in the core client/monitor graph. Separating them prevents, for example, a drag operation from requiring mutable access to configuration and pending launches merely because all three once lived in `Globals`.

`PendingLaunchTracker` should own queue-specific operations such as recording, matching, consuming, and pruning launches instead of exposing a raw `VecDeque` throughout the codebase.

## Dependency Rules

### Boundary code

The following code may receive `WmCtx` or backend-specific contexts:

- X11 and Wayland event handlers;
- action dispatch;
- IPC command entry points;
- startup, shutdown, and reload orchestration;
- backend-specific application code;
- operations whose purpose is explicitly to coordinate several runtime domains and backend effects.

Boundary functions should delegate quickly to narrower model or policy functions.

### Internal model and policy code

Internal functions should take direct references or stable domain inputs:

```rust
fn visibility_plan(
    monitors: &MonitorManager,
    clients: &ClientManager,
) -> Vec<VisibilityEntry>;

fn update_window_mode(
    clients: &mut ClientManager,
    win: WindowId,
    mode: BaseClientMode,
) -> Option<WindowModePlan>;

fn commit_view_selection(
    monitors: &mut MonitorManager,
    new_mask: TagMask,
) -> Option<MonitorId>;

impl WmModel {
    fn transfer_client(
        &mut self,
        win: WindowId,
        target: MonitorId,
    ) -> Option<ClientTransferOutcome>;
}
```

Use a named input structure only when three or more values repeatedly form a stable concept. Do not create one-use “context slices” solely to shorten argument lists.

### Context access

The final architecture removes `CoreCtx::globals()` and `CoreCtx::globals_mut()`.

It should not replace them with an equally unrestricted `model_mut()` accessor available to ordinary feature modules. During migration, temporary broad access may be necessary to keep each stage compiling, but it is not part of the final design.

`CoreCtx` may remain as an orchestration adapter that holds disjoint borrows from `Wm`. Its purpose is to support boundary code, scheduling, and reborrowing—not to be the default parameter for model calculations.

## Model Decisions and Backend Effects

Shared code should decide what must happen without performing backend I/O. The resulting value is then applied by X11 or Wayland code.

```diagram
╭──────────────────────╮
│ Event/action boundary│
╰──────────┬───────────╯
           ▼
╭──────────────────────╮
│ Narrow model operation│
│ mutate + return plan  │
╰──────────┬───────────╯
           ▼
╭──────────────────────╮
│ Backend-specific apply│
│ X11 or Wayland        │
╰──────────┬───────────╯
           ▼
╭──────────────────────╮
│ Queue/flush/sync work │
╰──────────────────────╯
```

The ordering of model mutation, backend updates, focus changes, layout scheduling, flushes, and compositor synchronization is observable behavior and must be preserved during the refactor.

### Plans versus capability traits

Use a plan or outcome when backends implement the same policy differently. Visibility is the clearest example:

- shared policy determines which clients are visible;
- X11 may move windows off-screen and update protocol state;
- Wayland maps or unmaps compositor surfaces.

Forcing both through one generic visibility-effect trait would hide meaningful differences. A shared `VisibilityEntry` plan with separate appliers is more explicit.

Use capability traits where both backends perform the same category of external operation, such as resizing, querying pointer position, or enumerating outputs. Capability traits should consume IDs, geometry, and plans where possible. A capability method that accepts the whole model has not narrowed the dependency.

The existing capability split should also be corrected where responsibilities are misplaced. For example, `window_protocol` is a window query and should not belong to `OutputOps`; it should move to `WindowOps` or a focused `WindowQuery` capability.

Do not create traits such as `HasClients`, `HasConfig`, or `CoreCapabilities` for state access. Those reproduce the god context behind indirection.

## Error and Invariant Handling

Model operations should represent expected absence explicitly with `Option` or a small outcome enum. Backend failures should remain at the effect boundary and follow the backend's existing error policy.

The refactor must preserve these invariants:

- a managed client has one authoritative monitor assignment;
- monitor focus and z-order lists do not gain duplicate client IDs;
- transfer and removal update all relevant model indexes together;
- selected monitor and tag-set indexes remain valid;
- backend effects use values from the completed model transition, not stale pre-transition snapshots;
- queued work identifies every monitor affected by a transition;
- X11 and Wayland effect ordering remains unchanged.

Do not introduce `RefCell`, `Mutex`, `Arc`, unsafe aliasing, or global mutable state to avoid borrow-checker errors. Borrow conflicts should be resolved by narrowing scopes, splitting references to independent fields, or returning plans before applying effects.

## Breaking Changes

Breaking internal changes are explicitly allowed. Expected breakage includes:

- removing or renaming `Globals`;
- moving fields from `Globals` into `Wm`, `WmModel`, or dedicated state owners;
- replacing `ctx.core().globals()` and `globals_mut()` calls;
- changing internal function signatures from `WmCtx` or `Globals` to direct dependencies;
- moving model-invariant helpers onto `WmModel`;
- changing private plan and outcome types to support exact tests;
- moving incorrectly grouped backend capability methods;
- updating tests and internal call sites across both backends.

Public IPC behavior, configuration formats, user-visible window-management policy, and supported backend behavior are not intended to break as part of this work. Any such behavior change should be isolated and justified separately rather than hidden inside the architecture migration.

## Migration Plan

### Stage 1: Establish model-level regression coverage

Before moving ownership, directly test the four model operations introduced by the current backend split:

1. `visibility_plan`;
2. `update_window_mode`;
3. `transfer_client_model`;
4. `commit_view_selection`.

The tests should assert both returned plans/outcomes and resulting model state. This realizes the backend-free testability promised by the extraction and protects behavior while signatures and ownership change.

Required coverage includes:

- visibility across active tags, inactive tags, hidden clients, sticky clients, and multiple monitors;
- preservation of geometry, border width, mode, and deterministic plan order;
- missing-client and floating/tiling mode transitions, including border and saved geometry behavior;
- transfer updates to source and target focus lists, z-order, tags, sticky state, monitor assignment, scratchpad handling, and arrange requirements;
- no-op and changed tag views, tag-set buffer switching, previous-tag history, and selected-monitor isolation.

This stage should not change runtime behavior.

### Stage 2: Narrow existing operations before moving ownership

Change model calculations to accept only the managers or configuration slices they use while `Globals` still exists. Start with the four tested functions, then continue through high-churn areas:

1. tag and view operations;
2. floating mode transitions;
3. visibility and layout planning;
4. monitor transfer and synchronization;
5. focus resolution;
6. mouse, drag, and Wayland input policy.

This stage creates architectural value immediately and prevents the ownership move from becoming a purely mechanical rename.

### Stage 3: Introduce `WmModel` and sibling runtime state

Move clients, monitors, and tags into `WmModel`. Move configuration, behavior, interaction, keyboard state, and pending launches into sibling fields owned by `Wm`.

Rename `Globals` only when the remaining type truthfully represents the window-manager model. Update construction and reload paths so state is built off to the side and installed coherently.

Configuration reload must continue to avoid exposing partially applied configuration. If useful, return a build result such as:

```rust
struct ReloadedState {
    config: RuntimeConfig,
    keyboard: KeyboardLayoutState,
    tag_template: Vec<TagNames>,
}
```

The exact type is optional; preserving atomic installation is required.

### Stage 4: Remove broad context access

Migrate remaining call sites and remove `CoreCtx::globals()` and `globals_mut()`. Work subsystem by subsystem so each intermediate commit compiles and tests pass.

Suggested order:

1. tags and view;
2. floating state;
3. visibility and layouts;
4. monitor operations;
5. focus;
6. mouse and drag interactions;
7. Wayland input and runtime callbacks;
8. IPC, reload, and startup.

At this stage, `WmCtx` remains valid at orchestration boundaries but should disappear from pure model calculations.

### Stage 5: Tighten backend capabilities

Audit backend traits after model dependencies are narrow:

- move methods to the capability that owns their responsibility;
- replace whole-model trait parameters with IDs, values, snapshots, or plans;
- retain explicit `WmCtx::X11` and `WmCtx::Wayland` branches where protocol mechanics differ;
- preserve flush, focus, map/unmap, and synchronization ordering.

### Stage 6: Remove transitional APIs and enforce the boundary

Delete temporary broad accessors, obsolete wrappers, aliases, and compatibility methods. Search the codebase for remaining `Globals`, unrestricted model access, and internal `WmCtx` signatures that can be narrowed.

The final cleanup should reduce API surface rather than leave both old and new access patterns indefinitely.

## Validation Strategy

Every migration stage should compile and run focused tests before proceeding. Verification should include:

- direct unit tests for model decisions and invariants;
- existing layout, monitor, client, tag, focus, and geometry tests;
- debug builds for the default X11 configuration and the Wayland feature set;
- backend-specific tests where available;
- formatting and lint checks used by the repository;
- manual smoke tests for focus, tag switching, floating transitions, monitor transfer, drag interactions, visibility, configuration reload, and startup on both backends when practical.

Release builds are intentionally excluded because repository guidance states they are too expensive for routine testing.

## Risks and Mitigations

### Backend effect ordering

Model extraction can accidentally reorder X11 property changes, focus, arranging, flushing, or Wayland space synchronization. Preserve ordering in boundary functions and review effect sequences explicitly.

### Client/monitor graph corruption

Directly borrowing managers can tempt callers to bypass aggregate operations. Keep transfer, attach/detach, monitor removal, and remapping behind `WmModel` operations where multiple indexes must change together. Test list membership and duplicate prevention.

### Reload inconsistency

Configuration, keyboard state, tag templates, and backend-derived metrics currently change in one orchestration path. Build replacement state before installation and keep backend application after the coherent state update.

### Cosmetic abstraction

More structs and accessors can increase code without reducing dependency reach. Judge each change by the resulting function signatures: if an internal function can still access all runtime state, the boundary has not improved.

### Forced backend unification

Sharing a policy does not imply sharing protocol mechanics. Keep separate appliers where X11 and Wayland differ, especially for visibility, focus, geometry acknowledgment, and compositor synchronization.

### Borrow-driven complexity

Do not respond to borrow errors with interior mutability or generic context traits. End model borrows before effects, return owned plans, and split direct borrows from independent `Wm` fields.

## Non-Goals

This architecture effort does not, by itself:

- change window-management policy;
- redesign the IPC protocol or configuration format;
- merge X11 and Wayland implementation details;
- make every field immutable;
- split every large file or large struct;
- add abstractions based on hypothetical future backends;
- fix unrelated policy quirks discovered during migration.

Nearby behavior bugs should be recorded and fixed separately unless they prevent the refactor or invalidate its regression tests.

## Completion Criteria

The re-architecture is complete when:

1. `Wm` is the clear composition root and `WmModel` owns the client/monitor/tag graph.
2. Configuration, interaction, keyboard, launch tracking, pending work, bar, and focus state have explicit sibling ownership.
3. `Globals` and unrestricted `CoreCtx` global access are removed.
4. Internal model functions declare narrow state dependencies and do not receive `WmCtx` by default.
5. Backend traits represent external capabilities rather than state access.
6. Backend-specific application remains explicit where X11 and Wayland semantics differ.
7. Cross-manager invariants are encapsulated and directly tested.
8. The four newly extracted model functions have meaningful direct unit coverage.
9. Both backend configurations compile and the relevant test suites pass without release builds.
10. Transitional wrappers and duplicate old/new APIs have been removed.

The success metric is not a smaller top-level struct. It is that a reader can understand what an operation depends on from its signature, model behavior can be tested without constructing a backend, and unrelated runtime domains can evolve without borrowing or coupling the entire window manager.

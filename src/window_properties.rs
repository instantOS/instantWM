//! Per-window properties chooser.
//!
//! The central [`crate::menu::MenuManager`] owns the presenter lifecycle. This
//! module snapshots the actions for one managed client and validates the
//! chosen action against its current state before applying it.
use crate::actions::NamedAction;
use crate::contexts::WmCtx;
use crate::core_state::WmCore;
use crate::menu::{ActiveMenu, MenuChoice, MenuColor, MenuOptions, MenuSpec, shortcut_hint};
use crate::types::{Client, ClientMode, ClientPlacement, Key, Point, WindowId};

pub(crate) struct WindowPropertiesState {
    win: WindowId,
    origin: MenuOrigin,
    choices: Vec<PropertyChoice>,
}

#[derive(Clone, Copy)]
enum MenuOrigin {
    Keyboard,
    Bar(Point),
}

#[derive(Debug, PartialEq, Eq)]
enum PropertyChoice {
    Lock(bool),
    Sticky(bool),
    Placement(ClientMode),
    FakePolicy(FullscreenState),
    ExitFullscreen(FullscreenState),
    RestoreScratchpad { name: String, visible: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FullscreenState {
    Windowed,
    Armed,
    Fake,
    Real,
}

impl FullscreenState {
    fn of(mode: ClientMode) -> Self {
        if mode.is_true_fullscreen() {
            Self::Real
        } else if mode.is_fake_fullscreen() {
            Self::Fake
        } else if mode.is_fake_armed() {
            Self::Armed
        } else {
            Self::Windowed
        }
    }
}

impl PropertyChoice {
    fn for_client(client: &Client) -> Vec<Self> {
        let mode = client.mode();
        let fullscreen = FullscreenState::of(mode);
        let mut choices = vec![Self::Lock(client.is_locked)];
        if !client.is_scratchpad() {
            choices.push(Self::Sticky(client.is_sticky));
            if fullscreen != FullscreenState::Real
                && !(client.is_fixed_size && mode.placement() == ClientPlacement::Floating)
            {
                choices.push(Self::Placement(mode));
            }
            choices.push(Self::FakePolicy(fullscreen));
        }
        if mode.is_fullscreen() {
            choices.push(Self::ExitFullscreen(fullscreen));
        }
        if let Some(scratchpad) = client.scratchpad() {
            choices.push(Self::RestoreScratchpad {
                name: scratchpad.name().to_string(),
                visible: client.is_scratchpad_visible(),
            });
        }
        choices
    }

    fn value(&self) -> &'static str {
        match self {
            Self::Lock(_) => "lock",
            Self::Sticky(_) => "sticky",
            Self::Placement(_) => "floating",
            Self::FakePolicy(_) => "fake-policy",
            Self::ExitFullscreen(_) => "fullscreen-exit",
            Self::RestoreScratchpad { .. } => "scratchpad-restore",
        }
    }

    fn row(&self, bindings: &[Key]) -> MenuChoice {
        let value = self.value();
        match self {
            Self::Lock(locked) => toggle_choice(
                value,
                if *locked {
                    "Unlock window (allow closing)"
                } else {
                    "Lock window (prevent closing)"
                },
                if *locked { "codlock" } else { "codunlock" },
                *locked,
                shortcut_hint(bindings, &NamedAction::ToggleLocked),
            ),
            Self::Sticky(sticky) => toggle_choice(
                value,
                if *sticky {
                    "Disable sticky (assigned tags only)"
                } else {
                    "Enable sticky (show on all tags)"
                },
                "codpin",
                *sticky,
                shortcut_hint(bindings, &NamedAction::ToggleSticky),
            ),
            Self::Placement(mode) => {
                let floating = mode.placement() == ClientPlacement::Floating;
                toggle_choice(
                    value,
                    if floating {
                        "Tile window"
                    } else {
                        "Float window"
                    },
                    "codwindow",
                    floating,
                    shortcut_hint(bindings, &NamedAction::ToggleFloating),
                )
            }
            Self::FakePolicy(state) => {
                let armed = matches!(state, FullscreenState::Armed | FullscreenState::Fake);
                toggle_choice(
                    value,
                    match state {
                        FullscreenState::Windowed => "Enable fake fullscreen for this window",
                        FullscreenState::Armed => "Disable fake fullscreen for this window",
                        FullscreenState::Fake => "Switch to real fullscreen",
                        FullscreenState::Real => "Switch to fake fullscreen",
                    },
                    "mdfullscreen",
                    armed,
                    shortcut_hint(bindings, &NamedAction::ToggleFakeFullscreen),
                )
            }
            Self::ExitFullscreen(_) => MenuChoice::item(value, "Exit fullscreen")
                .icon("mdfullscreen")
                .color(MenuColor::Blue),
            Self::RestoreScratchpad { name, .. } => MenuChoice::item(
                value,
                format!("Restore scratchpad {name} as a normal window"),
            )
            .icon("codlayers")
            .color(MenuColor::Blue),
        }
    }

    fn apply(&self, ctx: &mut WmCtx<'_>, win: WindowId) -> Result<(), String> {
        match self {
            Self::Lock(_) => crate::toggles::toggle_locked(ctx, win),
            Self::Sticky(_) => crate::toggles::toggle_sticky(ctx.core_mut(), win),
            Self::Placement(_) => crate::floating::toggle_client_floating(ctx, win),
            Self::FakePolicy(_) => {
                crate::client::fullscreen::toggle_client_fake_fullscreen(ctx, win)
            }
            Self::ExitFullscreen(_) => crate::client::set_fullscreen(ctx, win, false),
            Self::RestoreScratchpad { .. } => {
                crate::floating::scratchpad_restore(ctx, None, Some(win))?;
            }
        }
        Ok(())
    }
}

fn toggle_choice(
    value: &str,
    label: &str,
    icon: &'static str,
    on: bool,
    shortcut: Option<String>,
) -> MenuChoice {
    MenuChoice::item(value, label)
        .icon(icon)
        .color(if on {
            MenuColor::Green
        } else {
            MenuColor::Blue
        })
        .shortcut(shortcut)
}

fn menu_spec(
    core: &WmCore,
    bindings: &[Key],
    win: WindowId,
    origin: MenuOrigin,
) -> Option<(MenuSpec, WindowPropertiesState)> {
    let view = core.state.model.client_view(win)?;
    let client = view.client;
    let properties = PropertyChoice::for_client(client);
    let title = if client.title.trim().is_empty() {
        "Untitled window"
    } else {
        &client.title
    };
    let mut choices = vec![
        MenuChoice::heading(title)
            .icon("codwindowactive")
            .color(MenuColor::Blue),
    ];
    choices.extend(properties.iter().map(|property| property.row(bindings)));

    let mut options = MenuOptions {
        placeholder: Some("Find a window action".into()),
        width: Some("auto".into()),
        line_height: Some("auto".into()),
        lines: Some(choices.len() as u32),
        ..Default::default()
    };
    if let MenuOrigin::Bar(press) = origin {
        let monitor = view.monitor;
        let local_x = core
            .bar
            .monitor_hit_cache(monitor.id())
            .and_then(|hit| hit.title_ranges.iter().find(|range| range.win == win))
            .map_or_else(|| press.x - monitor.work_rect().x, |range| range.start);
        options.anchor_below_bar(monitor, local_x);
    }

    Some((
        MenuSpec { options, choices },
        WindowPropertiesState {
            win,
            origin,
            choices: properties,
        },
    ))
}

fn open_target(ctx: &mut WmCtx<'_>, win: WindowId, origin: MenuOrigin) -> Result<(), String> {
    let core = ctx.core_mut();
    let Some((spec, state)) = menu_spec(core, &core.state.config.bindings.keys, win, origin) else {
        return Ok(());
    };
    core.menus.start(spec, ActiveMenu::WindowProperties(state))
}

pub(crate) fn open_from_title(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    press: Point,
) -> Result<(), String> {
    open_target(ctx, win, MenuOrigin::Bar(press))
}

pub(crate) fn open(ctx: &mut WmCtx<'_>) -> Result<(), String> {
    let Some(win) = ctx.core().state.model.selected_win() else {
        return Ok(());
    };
    open_target(ctx, win, MenuOrigin::Keyboard)
}

pub(crate) fn apply(
    ctx: &mut WmCtx<'_>,
    state: WindowPropertiesState,
    answer: &str,
) -> Result<(), String> {
    let WindowPropertiesState {
        win,
        origin,
        choices,
    } = state;
    let Some(choice) = choices.iter().find(|choice| choice.value() == answer) else {
        return Ok(());
    };
    let model = &ctx.core().state.model;
    if matches!(origin, MenuOrigin::Keyboard) && model.selected_win() != Some(win) {
        return Ok(());
    }
    let Some(client) = model.client(win) else {
        return Ok(());
    };
    if PropertyChoice::for_client(client).contains(choice) {
        choice.apply(ctx, win)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendKind;
    use crate::config::keybindings::default_keybinds;
    use crate::test_support::{MonitorBuilder, TestWm, add_selected_client};
    use crate::types::{ClientMode, Rect, TagMask};

    fn fixture_with(mode: ClientMode, configure: impl FnOnce(&mut Client)) -> (TestWm, WindowId) {
        let mut wm = TestWm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.model.tags.num_tags = 3;
        let monitor_id = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .named("TEST-1")
                .monitor_rect(Rect::new(0, 0, 1200, 800))
                .tag_count(3)
                .selected_tags(TagMask::single(1).unwrap())
                .build(),
        );
        wm.core.state.model.monitors.set_selected(monitor_id);

        let win = WindowId(7);
        let mut client = Client {
            win,
            title: "Editor".into(),
            tags: TagMask::single(1).unwrap(),
            border_width: 2,
            old_border_width: 2,
            ..Client::default()
        };
        client.set_mode_for_test(mode);
        configure(&mut client);
        add_selected_client(&mut wm.core.state.model, monitor_id, client);
        (wm, win)
    }

    fn fixture(mode: ClientMode) -> (TestWm, WindowId) {
        fixture_with(mode, |_| {})
    }

    fn scratchpad_fixture(visible: bool) -> (TestWm, WindowId) {
        let mut wm = TestWm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.model.tags.num_tags = 3;
        let monitor_id = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .named("TEST-1")
                .tag_count(3)
                .selected_tags(TagMask::single(1).unwrap())
                .build(),
        );
        wm.core.state.model.monitors.set_selected(monitor_id);

        let win = WindowId(9);
        let mut client = Client {
            win,
            title: "Notes".into(),
            tags: TagMask::single(1).unwrap(),
            border_width: 2,
            old_border_width: 2,
            ..Client::default()
        };
        client
            .promote_to_scratchpad(monitor_id, "notes", None, 1200, 800)
            .unwrap();
        client.is_hidden = !visible;
        add_selected_client(&mut wm.core.state.model, monitor_id, client);
        (wm, win)
    }

    fn menu_input(wm: &TestWm, win: WindowId) -> String {
        let bindings = default_keybinds(BackendKind::Wayland);
        menu_spec(&wm.core, &bindings, win, MenuOrigin::Keyboard)
            .expect("fixture window is managed")
            .0
            .input()
    }

    fn row<'a>(input: &'a str, value: &str) -> Option<&'a str> {
        input.lines().find(|line| {
            let metadata = line
                .strip_prefix('{')
                .and_then(|line| line.split_once('}'))
                .map(|(metadata, _)| metadata)
                .unwrap_or_default();
            metadata
                .split_whitespace()
                .any(|token| token == format!("value={value}"))
        })
    }

    #[test]
    fn keyboard_menu_centers_and_sizes_to_the_actions() {
        let (wm, win) = fixture(ClientMode::tiled());
        let (spec, _) = menu_spec(&wm.core, &[], win, MenuOrigin::Keyboard).unwrap();
        let rows = spec.choices.len().to_string();
        let command = spec.command();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        for pair in [
            ["--placeholder", "Find a window action"],
            ["--position", "center"],
            ["--width", "auto"],
            ["--line-height", "auto"],
            ["--lines", rows.as_str()],
        ] {
            assert!(args.windows(2).any(|args| args == pair), "missing {pair:?}");
        }
    }

    #[test]
    fn title_menu_uses_the_clicked_cells_output_and_left_edge() {
        let (mut wm, win) = fixture(ClientMode::tiled());
        let id = wm.core.state.model.selected_monitor_id();
        let monitor = wm.core.state.model.monitor_mut(id).unwrap();
        monitor.num = 1;
        monitor.monitor_rect = Rect::new(-1920, -200, 1920, 1080);
        monitor.available_rect = Rect::new(-1900, -190, 1900, 1070);
        monitor.bar_height = 36;
        wm.core.bar.replace_hit_cache(
            id,
            crate::bar::MonitorHitCache {
                title_ranges: vec![crate::bar::TitleHitRange {
                    win,
                    start: 300,
                    end: 600,
                }],
                ..Default::default()
            },
        );
        let (spec, _) =
            menu_spec(&wm.core, &[], win, MenuOrigin::Bar(Point::new(-1550, -180))).unwrap();
        assert_eq!(spec.options.position, crate::menu::MenuPosition::TopLeft);
        assert_eq!(spec.options.monitor.as_deref(), Some("1"));
        assert_eq!(spec.options.x_offset, Some(320));
        assert_eq!(spec.options.y_offset, Some(46));
    }

    #[test]
    fn ordinary_menu_shows_a_title_and_four_actions_with_shortcuts() {
        let (wm, win) = fixture(ClientMode::tiled());
        let input = menu_input(&wm, win);
        assert_eq!(input.lines().count(), 5);
        assert_eq!(
            input.lines().next(),
            Some("{heading blue icon=codwindowactive} Editor")
        );
        for (value, expected) in [
            (
                "lock",
                "{blue icon=codunlock value=lock} Lock window (prevent closing)",
            ),
            (
                "sticky",
                "{blue icon=codpin value=sticky shortcut=super-ctrl-s} Enable sticky (show on all tags)",
            ),
            (
                "floating",
                "{blue icon=codwindow value=floating shortcut=super-shift-space} Float window",
            ),
            (
                "fake-policy",
                "{blue icon=mdfullscreen value=fake-policy shortcut=super-shift-f} Enable fake fullscreen for this window",
            ),
        ] {
            assert_eq!(row(&input, value), Some(expected));
        }
    }

    #[test]
    fn enabled_properties_describe_the_reverse_action() {
        let (wm, win) = fixture_with(ClientMode::floating().as_fake_armed(), |client| {
            client.is_locked = true;
            client.is_sticky = true;
        });
        let input = menu_input(&wm, win);
        for (value, label) in [
            ("lock", "Unlock window (allow closing)"),
            ("sticky", "Disable sticky (assigned tags only)"),
            ("floating", "Tile window"),
            ("fake-policy", "Disable fake fullscreen for this window"),
        ] {
            let line = row(&input, value).unwrap();
            assert!(line.starts_with("{green"));
            assert!(line.ends_with(label));
        }
    }

    #[test]
    fn fullscreen_menu_offers_switching_modes_or_exiting() {
        for (mode, label, has_placement) in [
            (
                ClientMode::tiled().as_fake_fullscreen(),
                "Switch to real fullscreen",
                true,
            ),
            (
                ClientMode::tiled().as_fullscreen(),
                "Switch to fake fullscreen",
                false,
            ),
        ] {
            let (wm, win) = fixture(mode);
            let input = menu_input(&wm, win);
            assert!(row(&input, "fake-policy").unwrap().ends_with(label));
            assert!(
                row(&input, "fullscreen-exit")
                    .unwrap()
                    .ends_with("Exit fullscreen")
            );
            assert_eq!(row(&input, "floating").is_some(), has_placement);
        }
    }

    #[test]
    fn scratchpad_rows_offer_restore_without_ordinary_toggles() {
        for visible in [false, true] {
            let (wm, win) = scratchpad_fixture(visible);
            let input = menu_input(&wm, win);
            assert!(
                row(&input, "scratchpad-restore")
                    .unwrap()
                    .ends_with("Restore scratchpad notes as a normal window")
            );
            assert!(row(&input, "lock").is_some());
            assert!(row(&input, "sticky").is_none());
            assert!(row(&input, "floating").is_none());
            assert!(row(&input, "fake-policy").is_none());
        }
    }

    fn snapshot(wm: &TestWm, win: WindowId) -> WindowPropertiesState {
        menu_spec(&wm.core, &[], win, MenuOrigin::Keyboard)
            .unwrap()
            .1
    }

    #[test]
    fn title_menu_actions_target_the_clicked_window_even_when_it_is_unfocused() {
        let (mut wm, win) = fixture(ClientMode::tiled());
        let monitor_id = wm.core.state.model.selected_monitor_id();
        let other = WindowId(8);
        add_selected_client(
            &mut wm.core.state.model,
            monitor_id,
            Client {
                win: other,
                tags: TagMask::single(1).unwrap(),
                ..Client::default()
            },
        );
        for answer in ["fake-policy", "floating", "sticky"] {
            let (_, state) =
                menu_spec(&wm.core, &[], win, MenuOrigin::Bar(Point::new(0, 0))).unwrap();
            wm.with_ctx(|ctx| apply(ctx, state, answer)).unwrap();
        }
        let model = &wm.core.state.model;
        let client = model.client(win).unwrap();
        assert!(client.mode().is_fake_armed());
        assert!(client.mode().is_normal_floating());
        assert!(client.is_sticky);
        let client = model.client(other).unwrap();
        assert_eq!(client.mode(), ClientMode::tiled());
        assert!(!client.is_sticky);
        assert_eq!(model.selected_win(), Some(other));
    }

    #[test]
    fn apply_toggles_lock_and_sticky() {
        let (mut wm, win) = fixture(ClientMode::tiled());
        for expected in [true, false] {
            for answer in ["lock", "sticky"] {
                let state = snapshot(&wm, win);
                wm.with_ctx(|ctx| apply(ctx, state, answer)).unwrap();
            }
            let client = wm.core.state.model.client(win).unwrap();
            assert_eq!(client.is_locked, expected);
            assert_eq!(client.is_sticky, expected);
        }
    }

    #[test]
    fn apply_ignores_dismissal_and_values_missing_from_the_snapshot() {
        let (mut wm, win) = fixture(ClientMode::tiled());
        let state = snapshot(&wm, win);
        wm.core
            .state
            .model
            .client_mut(win)
            .unwrap()
            .enter_fullscreen();
        wm.with_ctx(|ctx| apply(ctx, state, "fullscreen-exit"))
            .unwrap();
        assert!(
            wm.core
                .state
                .model
                .client(win)
                .unwrap()
                .mode()
                .is_true_fullscreen()
        );

        for answer in ["", "arbitrary typed text"] {
            let state = snapshot(&wm, win);
            wm.with_ctx(|ctx| apply(ctx, state, answer)).unwrap();
        }
        assert!(!wm.core.state.model.client(win).unwrap().is_locked);
    }

    #[test]
    fn a_stale_fake_policy_choice_does_not_demote_real_fullscreen() {
        let (mut wm, win) = fixture(ClientMode::tiled());
        let state = snapshot(&wm, win);
        wm.core
            .state
            .model
            .client_mut(win)
            .unwrap()
            .enter_fullscreen();
        wm.with_ctx(|ctx| apply(ctx, state, "fake-policy")).unwrap();
        assert!(
            wm.core
                .state
                .model
                .client(win)
                .unwrap()
                .mode()
                .is_true_fullscreen()
        );
    }

    #[test]
    fn apply_ignores_stale_selection() {
        let (mut wm, win) = fixture(ClientMode::tiled());
        let state = snapshot(&wm, win);
        let monitor_id = wm.core.state.model.selected_monitor_id();
        let other = WindowId(8);
        add_selected_client(
            &mut wm.core.state.model,
            monitor_id,
            Client {
                win: other,
                tags: TagMask::single(1).unwrap(),
                ..Client::default()
            },
        );
        wm.with_ctx(|ctx| apply(ctx, state, "lock")).unwrap();
        assert!(!wm.core.state.model.client(win).unwrap().is_locked);
        assert!(!wm.core.state.model.client(other).unwrap().is_locked);
    }
}

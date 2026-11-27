//! Per-window properties chooser.
//!
//! The central [`crate::menu::MenuManager`] owns the presenter lifecycle. This
//! module snapshots one managed client into headings and toggles, then applies
//! a validated answer only while that window is still the selected one.
use crate::actions::NamedAction;
use crate::contexts::WmCtx;
use crate::menu::{
    ActiveMenu, MenuChoice, MenuColor, MenuOptions, MenuPosition, MenuSpec, shortcut_hint,
};
use crate::model::WmModel;
use crate::types::{Client, ClientMode, ClientPlacement, Key, TagMask, WindowId};

pub(crate) struct WindowPropertiesState {
    win: WindowId,
    choices: Vec<PropertyChoice>,
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
                format!("Locked: {}", yes_no(*locked)),
                if *locked { "codlock" } else { "codunlock" },
                *locked,
                shortcut_hint(bindings, &NamedAction::ToggleLocked),
            ),
            Self::Sticky(sticky) => toggle_choice(
                value,
                format!("Sticky: {}", yes_no(*sticky)),
                "codpin",
                *sticky,
                shortcut_hint(bindings, &NamedAction::ToggleSticky),
            ),
            Self::Placement(mode) => {
                let floating = mode.placement() == ClientPlacement::Floating;
                toggle_choice(
                    value,
                    format!("Placement: {}", if floating { "floating" } else { "tiled" }),
                    "codwindow",
                    floating,
                    shortcut_hint(bindings, &NamedAction::ToggleFloating),
                )
            }
            Self::FakePolicy(state) => {
                let armed = matches!(state, FullscreenState::Armed | FullscreenState::Fake);
                toggle_choice(
                    value,
                    format!(
                        "Fake fullscreen policy: {}",
                        if armed { "armed" } else { "disarmed" }
                    ),
                    "mdfullscreen",
                    armed,
                    shortcut_hint(bindings, &NamedAction::ToggleFakeFullscreen),
                )
            }
            Self::ExitFullscreen(state) => MenuChoice::item(
                value,
                format!(
                    "Exit fullscreen: {}",
                    if *state == FullscreenState::Fake {
                        "fake"
                    } else {
                        "real"
                    }
                ),
            )
            .icon("mdfullscreen")
            .color(MenuColor::Yellow),
            Self::RestoreScratchpad { name, visible } => MenuChoice::item(
                value,
                format!(
                    "Scratchpad: {name} ({}) — restore",
                    if *visible { "visible" } else { "hidden" }
                ),
            )
            .icon("codlayers")
            .color(MenuColor::Green),
        }
    }

    fn apply(&self, ctx: &mut WmCtx<'_>, win: WindowId) -> Result<(), String> {
        match self {
            Self::Lock(_) => crate::toggles::toggle_locked(ctx, win),
            Self::Sticky(_) => crate::toggles::toggle_sticky(ctx.core_mut(), win),
            Self::Placement(_) => crate::floating::toggle_floating(ctx),
            Self::FakePolicy(_) => crate::client::fullscreen::toggle_fake_fullscreen(ctx),
            Self::ExitFullscreen(_) => crate::client::set_fullscreen(ctx, win, false),
            Self::RestoreScratchpad { .. } => {
                crate::floating::scratchpad_restore(ctx, None, Some(win))?;
            }
        }
        Ok(())
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn tags_label(tags: TagMask) -> String {
    let tags: Vec<_> = tags
        .without_scratchpad()
        .iter()
        .map(|tag| tag.to_string())
        .collect();
    if tags.is_empty() {
        "-".to_string()
    } else {
        tags.join(",")
    }
}

fn flags_label(client: &Client) -> Option<String> {
    let flags = [
        (client.is_fixed_size, "fixed size"),
        (client.never_focus, "never focus"),
        (client.keep_above, "always above"),
        (client.is_borderless, "borderless"),
        (client.is_modal, "modal"),
        (client.transient_for.is_some(), "transient"),
        (client.is_urgent, "urgent"),
    ];
    let flags: Vec<_> = flags
        .into_iter()
        .filter_map(|(set, label)| set.then_some(label))
        .collect();
    (!flags.is_empty()).then(|| flags.join(" · "))
}

fn toggle_choice(
    value: &str,
    label: String,
    icon: &'static str,
    on: bool,
    shortcut: Option<String>,
) -> MenuChoice {
    MenuChoice::item(value, label)
        .icon(icon)
        .color(if on {
            MenuColor::Green
        } else {
            MenuColor::Fade
        })
        .shortcut(shortcut)
}

fn menu_spec(
    model: &WmModel,
    bindings: &[Key],
    win: WindowId,
    preselect: Option<String>,
) -> Option<(MenuSpec, WindowPropertiesState)> {
    let view = model.client_view(win)?;
    let client = view.client;
    let scratchpad = client.is_scratchpad();
    let properties = PropertyChoice::for_client(client);
    let mut choices = vec![
        MenuChoice::heading(client.title.clone())
            .icon("codwindowactive")
            .color(MenuColor::Blue),
        MenuChoice::heading(format!(
            "#{} · {} · tags {} · border {}px",
            win,
            view.monitor.name,
            tags_label(client.tags),
            client.border_width
        ))
        .color(MenuColor::Fade),
        MenuChoice::heading("State").color(MenuColor::Blue),
    ];

    choices.extend(properties.iter().map(|property| property.row(bindings)));

    choices.push(MenuChoice::heading("Info").color(MenuColor::Blue));
    choices.push(MenuChoice::heading(if scratchpad {
        "Tags: scratchpad role".to_string()
    } else {
        format!("Tags: {}", tags_label(client.tags))
    }));
    choices.push(MenuChoice::heading(format!(
        "Border: {} px",
        client.border_width
    )));
    if !scratchpad {
        choices.push(MenuChoice::heading("Scratchpad: none"));
    }
    if let Some(flags) = flags_label(client) {
        choices.push(MenuChoice::heading(format!("Client: {flags}")));
    }

    let spec = MenuSpec {
        options: MenuOptions {
            prompt: Some("window".into()),
            position: MenuPosition::Center,
            width: Some("auto".into()),
            line_height: Some("auto".into()),
            lines: Some(choices.len() as u32),
            preselect,
            ..Default::default()
        },
        choices,
    };
    Some((
        spec,
        WindowPropertiesState {
            win,
            choices: properties,
        },
    ))
}

fn open_target(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    preselect: Option<String>,
) -> Result<(), String> {
    let spec = {
        let core = ctx.core();
        menu_spec(
            &core.state.model,
            &core.state.config.bindings.keys,
            win,
            preselect,
        )
    };
    let Some((spec, state)) = spec else {
        return Ok(());
    };
    ctx.core_mut()
        .menus
        .start(spec, ActiveMenu::WindowProperties(state))
}

pub(crate) fn open(ctx: &mut WmCtx<'_>) -> Result<(), String> {
    let Some(win) = ctx.core().state.model.selected_win() else {
        return Ok(());
    };
    open_target(ctx, win, None)
}

pub(crate) fn apply(
    ctx: &mut WmCtx<'_>,
    state: WindowPropertiesState,
    answer: &str,
) -> Result<(), String> {
    let WindowPropertiesState { win, choices } = state;
    let Some(choice) = choices.iter().find(|choice| choice.value() == answer) else {
        return Ok(());
    };
    let model = &ctx.core().state.model;
    if model.selected_win() != Some(win) {
        return Ok(());
    }
    let Some(client) = model.client(win) else {
        return Ok(());
    };
    if PropertyChoice::for_client(client).contains(choice) {
        choice.apply(ctx, win)?;
    }
    if ctx.core().state.model.selected_win() == Some(win)
        && let Err(error) = open_target(ctx, win, Some(answer.to_string()))
    {
        log::warn!("window properties: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendKind;
    use crate::config::keybindings::default_keybinds;
    use crate::test_support::{MonitorBuilder, TestWm, add_selected_client};
    use crate::types::ClientMode;

    fn fixture_with(mode: ClientMode, configure: impl FnOnce(&mut Client)) -> (TestWm, WindowId) {
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
        menu_spec(&wm.core.state.model, &bindings, win, None)
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
    fn presenter_is_centered_auto_sized_and_preselects_a_choice() {
        let (wm, win) = fixture(ClientMode::tiled());
        let bindings = default_keybinds(BackendKind::Wayland);
        let (spec, _) = menu_spec(
            &wm.core.state.model,
            &bindings,
            win,
            Some("sticky".to_string()),
        )
        .unwrap();
        let rows = spec.choices.len().to_string();
        let command = spec.command();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        for pair in [
            ["--prompt", "window"],
            ["--position", "center"],
            ["--width", "auto"],
            ["--line-height", "auto"],
            ["--lines", rows.as_str()],
            ["--preselect", "sticky"],
        ] {
            assert!(args.windows(2).any(|args| args == pair), "missing {pair:?}");
        }
    }

    #[test]
    fn ordinary_window_rows_name_the_state_and_offer_every_toggle() {
        let (wm, win) = fixture(ClientMode::tiled());
        let input = menu_input(&wm, win);

        assert!(input.contains("{heading blue icon=codwindowactive} Editor"));
        assert!(input.contains("{heading fade} #7 · TEST-1 · tags 1 · border 2px"));
        assert!(input.contains("{heading blue} State"));
        assert_eq!(
            row(&input, "lock").unwrap(),
            "{fade icon=codunlock value=lock} Locked: no"
        );
        assert_eq!(
            row(&input, "sticky").unwrap(),
            "{fade icon=codpin value=sticky shortcut=super-ctrl-s} Sticky: no"
        );
        assert_eq!(
            row(&input, "floating").unwrap(),
            "{fade icon=codwindow value=floating shortcut=super-shift-space} Placement: tiled"
        );
        assert_eq!(
            row(&input, "fake-policy").unwrap(),
            "{fade icon=mdfullscreen value=fake-policy shortcut=super-shift-f} Fake fullscreen policy: disarmed"
        );
        assert!(row(&input, "fullscreen-exit").is_none());
        assert!(input.contains("{heading blue} Info"));
        assert!(input.contains("{heading} Tags: 1"));
        assert!(input.contains("{heading} Border: 2 px"));
        assert!(input.contains("{heading} Scratchpad: none"));
    }

    #[test]
    fn locked_sticky_and_armed_rows_are_green() {
        let (wm, win) = fixture_with(ClientMode::tiled().as_fake_armed(), |client| {
            client.is_locked = true;
            client.is_sticky = true;
        });
        let input = menu_input(&wm, win);

        assert_eq!(
            row(&input, "lock").unwrap(),
            "{green icon=codlock value=lock} Locked: yes"
        );
        assert_eq!(
            row(&input, "sticky").unwrap(),
            "{green icon=codpin value=sticky shortcut=super-ctrl-s} Sticky: yes"
        );
        assert_eq!(
            row(&input, "fake-policy").unwrap(),
            "{green icon=mdfullscreen value=fake-policy shortcut=super-shift-f} Fake fullscreen policy: armed"
        );
    }

    #[test]
    fn active_fake_fullscreen_counts_as_armed_and_keeps_placement() {
        let (wm, win) = fixture(ClientMode::tiled().as_fake_fullscreen());
        let input = menu_input(&wm, win);

        let policy = row(&input, "fake-policy").unwrap();
        assert!(policy.starts_with("{green"));
        assert!(policy.ends_with("Fake fullscreen policy: armed"));
        assert!(row(&input, "floating").is_some());
        assert_eq!(
            row(&input, "fullscreen-exit").unwrap(),
            "{yellow icon=mdfullscreen value=fullscreen-exit} Exit fullscreen: fake"
        );
    }

    #[test]
    fn true_fullscreen_reports_real_and_hides_the_placement_toggle() {
        let (wm, win) = fixture(ClientMode::tiled().as_fullscreen());
        let input = menu_input(&wm, win);

        assert_eq!(
            row(&input, "fullscreen-exit").unwrap(),
            "{yellow icon=mdfullscreen value=fullscreen-exit} Exit fullscreen: real"
        );
        assert!(row(&input, "floating").is_none());
    }

    #[test]
    fn scratchpad_rows_offer_restore_without_ordinary_toggles() {
        let (wm, win) = scratchpad_fixture(false);
        let input = menu_input(&wm, win);

        assert_eq!(
            row(&input, "scratchpad-restore").unwrap(),
            "{green icon=codlayers value=scratchpad-restore} Scratchpad: notes (hidden) — restore"
        );
        assert!(row(&input, "lock").is_some());
        assert!(row(&input, "sticky").is_none());
        assert!(row(&input, "floating").is_none());
        assert!(row(&input, "fake-policy").is_none());
        assert!(input.contains("{heading} Tags: scratchpad role"));
        assert!(!input.contains("Scratchpad: none"));
    }

    #[test]
    fn visible_scratchpads_report_their_visibility() {
        let (wm, win) = scratchpad_fixture(true);
        let input = menu_input(&wm, win);

        assert!(
            row(&input, "scratchpad-restore")
                .unwrap()
                .ends_with("Scratchpad: notes (visible) — restore")
        );
    }

    #[test]
    fn info_headings_carry_no_value_and_are_not_selectable() {
        let (wm, win) = fixture_with(ClientMode::tiled(), |client| {
            client.is_fixed_size = true;
            client.never_focus = false;
            client.keep_above = true;
            client.is_borderless = false;
            client.is_modal = false;
            client.is_urgent = true;
        });
        let input = menu_input(&wm, win);

        for line in input.lines().filter(|line| line.starts_with("{heading")) {
            assert!(!line.contains("value="), "{line}");
        }
        for label in [
            "Tags: 1",
            "Border: 2 px",
            "Scratchpad: none",
            "Client: fixed size · always above · urgent",
        ] {
            let line = input
                .lines()
                .find(|line| line.ends_with(label))
                .unwrap_or_else(|| panic!("missing {label}"));
            assert!(line.starts_with("{heading}"), "{line}");
            assert!(!line.contains("value="), "{line}");
        }
    }

    fn snapshot(wm: &TestWm, win: WindowId) -> WindowPropertiesState {
        menu_spec(&wm.core.state.model, &[], win, None).unwrap().1
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

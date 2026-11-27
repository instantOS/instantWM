//! WM-owned layout chooser. The central [`crate::menu::MenuManager`] owns the
//! presenter lifecycle; this module snapshots the chooser and applies a
//! validated answer.
use crate::actions::NamedAction;
use crate::contexts::WmCtx;
use crate::layouts::LayoutCommand;
use crate::menu::{
    ActiveMenu, MenuChoice, MenuColor, MenuOptions, MenuPosition, MenuSpec, shortcut_hint_where,
};
use crate::types::{Key, Monitor, MonitorId, TagMask};

pub(crate) struct Choice {
    value: String,
    label: String,
    icon: &'static str,
    color: MenuColor,
    action: Option<NamedAction>,
    shortcut: Option<String>,
}

impl Choice {
    fn menu_choice(&self) -> MenuChoice {
        MenuChoice::item(self.value.clone(), self.label.clone())
            .icon(self.icon)
            .color(self.color)
            .shortcut(self.shortcut.clone())
    }
}

fn layout_choice(layout: LayoutCommand, active: LayoutCommand) -> Choice {
    let label = match layout {
        LayoutCommand::Tile => "Main window on left",
        LayoutCommand::Grid => "Grid",
        LayoutCommand::BottomStack => "Main window on top",
        LayoutCommand::Floating => "Floating windows",
        LayoutCommand::Maximized => "Maximized windows",
    };
    let current = layout == active;
    Choice {
        value: layout.name().into(),
        label: format!("{label}{}", if current { " (current)" } else { "" }),
        icon: layout.symbol(),
        color: if current {
            MenuColor::Green
        } else {
            MenuColor::Blue
        },
        // Choosing the current layout must not silently reset edits.
        action: (!current).then_some(NamedAction::SetLayout(layout)),
        shortcut: None,
    }
}

fn choices(active: LayoutCommand) -> Vec<Choice> {
    // instantMENU fills columns top to bottom. Each tiling layout has its
    // reset to the right; floating and maximized share the final row since
    // they have no independent arrangement to reset.
    let layouts = [
        LayoutCommand::Tile,
        LayoutCommand::Grid,
        LayoutCommand::BottomStack,
    ];
    let mut entries: Vec<_> = layouts
        .iter()
        .map(|&layout| layout_choice(layout, active))
        .collect();
    entries.push(layout_choice(LayoutCommand::Floating, active));
    for layout in layouts {
        let choice = layout_choice(layout, active);
        entries.push(Choice {
            value: format!("reset-{}", layout.name()),
            label: format!(
                "Reset {}",
                choice.label.trim_end_matches(" (current)").to_lowercase()
            ),
            icon: "fa-arrow-rotate-left",
            color: MenuColor::Yellow,
            action: Some(NamedAction::ResetLayoutTo(layout)),
            shortcut: None,
        });
    }
    entries.push(layout_choice(LayoutCommand::Maximized, active));
    entries
}

fn selected_action(entries: &[Choice], answer: &str) -> Option<NamedAction> {
    entries
        .iter()
        .find(|entry| entry.value == answer)
        .and_then(|entry| entry.action.clone())
}

fn apply_shortcuts(entries: &mut [Choice], bindings: &[Key]) {
    for entry in entries {
        let action = entry
            .action
            .clone()
            .or_else(|| LayoutCommand::from_name(&entry.value).map(NamedAction::SetLayout));
        let Some(action) = action else { continue };
        entry.shortcut = shortcut_hint_where(bindings, |bound| {
            bound == &action
                || matches!(
                    (&action, bound),
                    (
                        NamedAction::SetLayout(LayoutCommand::Floating),
                        NamedAction::LayoutFloat
                    ) | (
                        NamedAction::SetLayout(LayoutCommand::Maximized),
                        NamedAction::ToggleTilingMaximized
                    )
                )
        });
    }
}

#[derive(Default)]
struct MenuAnchor {
    monitor: Option<u32>,
    x_offset: i32,
    y_offset: i32,
}

impl MenuAnchor {
    fn menu_options(&self, active: LayoutCommand) -> MenuOptions {
        MenuOptions {
            placeholder: Some("Choose window layout".into()),
            position: if self.monitor.is_some() {
                MenuPosition::TopLeft
            } else {
                MenuPosition::Center
            },
            monitor: Some(
                self.monitor
                    .map_or_else(|| "auto".into(), |index| index.to_string()),
            ),
            x_offset: self.x_offset,
            y_offset: self.y_offset,
            width: Some("auto".into()),
            line_height: Some("auto".into()),
            columns: Some(2),
            lines: Some(4),
            preselect: Some(active.name().into()),
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MenuContext {
    monitor: MonitorId,
    tags: TagMask,
}

impl MenuContext {
    fn from_monitor(monitor: &Monitor) -> Self {
        Self {
            monitor: monitor.id(),
            tags: monitor.visible_tags(),
        }
    }
}

/// Snapshot captured when the chooser opened. Only a value from this list may
/// be applied, and only while the selected monitor and tags still match.
pub(crate) struct LayoutMenuState {
    context: MenuContext,
    entries: Vec<Choice>,
}

fn anchor(monitor: &Monitor, layout_start: i32) -> (i32, i32) {
    (
        monitor.work_rect().x - monitor.monitor_rect.x + layout_start,
        monitor.bar_y() + monitor.bar_height - monitor.monitor_rect.y,
    )
}

fn menu_spec(active: LayoutCommand, anchor: &MenuAnchor, entries: &[Choice]) -> MenuSpec {
    MenuSpec {
        options: anchor.menu_options(active),
        choices: entries.iter().map(Choice::menu_choice).collect(),
    }
}

pub(crate) fn open(ctx: &mut WmCtx<'_>) -> Result<(), String> {
    let core = ctx.core_mut();
    let (context, active, anchor) = {
        let monitor = core.state.model.expect_selected_monitor();
        let context = MenuContext::from_monitor(monitor);
        let active = monitor.current_layout_command();
        let anchor = core
            .bar
            .monitor_hit_cache(monitor.id())
            .map(|hit| {
                let (x_offset, y_offset) = anchor(monitor, hit.layout_start);
                MenuAnchor {
                    monitor: Some(monitor.num.max(0) as u32),
                    x_offset,
                    y_offset,
                }
            })
            .unwrap_or_default();
        (context, active, anchor)
    };
    let mut entries = choices(active);
    apply_shortcuts(&mut entries, &core.state.config.bindings.keys);
    let spec = menu_spec(active, &anchor, &entries);
    core.menus.start(
        spec,
        ActiveMenu::Layout(LayoutMenuState { context, entries }),
    )
}

/// Apply a presenter answer against the snapshot taken when the menu opened.
pub(crate) fn apply(
    ctx: &mut WmCtx<'_>,
    state: LayoutMenuState,
    answer: &str,
) -> Result<(), String> {
    let LayoutMenuState { context, entries } = state;
    let unchanged = {
        let monitor = ctx.core().state.model.expect_selected_monitor();
        context == MenuContext::from_monitor(monitor)
    };
    if !unchanged {
        return Ok(());
    }
    let Some(action) = selected_action(&entries, answer) else {
        return Ok(());
    };
    let current = ctx
        .core()
        .state
        .model
        .expect_selected_monitor()
        .current_layout_command();
    if matches!(action, NamedAction::SetLayout(layout) if layout == current) {
        return Ok(());
    }
    action.execute(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu_presenter;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    fn wm_with_monitor() -> crate::test_support::TestWm {
        let mut wm =
            crate::test_support::TestWm::new(crate::backend::WaylandBackendData::default());
        let id = wm.core.state.model.monitors.push(
            crate::test_support::MonitorBuilder::new()
                .tag_count(2)
                .selected_tags(TagMask::single(1).unwrap())
                .build(),
        );
        wm.core.state.model.monitors.set_selected(id);
        wm
    }

    /// The chooser as `open` would have built it, including the default
    /// bindings that give every entry its shortcut hint.
    fn session(wm: &crate::test_support::TestWm) -> (MenuSpec, LayoutMenuState) {
        let monitor = wm.core.state.model.expect_selected_monitor();
        let context = MenuContext::from_monitor(monitor);
        let active = monitor.current_layout_command();
        let mut entries = choices(active);
        apply_shortcuts(
            &mut entries,
            &crate::config::keybindings::default_keybinds(crate::backend::BackendKind::Wayland),
        );
        let spec = menu_spec(active, &MenuAnchor::default(), &entries);
        (spec, LayoutMenuState { context, entries })
    }

    #[test]
    fn presenter_receives_shortcuts_and_returns_an_action_without_ipc() {
        let mut wm = wm_with_monitor();
        let mut event_loop = calloop::EventLoop::try_new().unwrap();
        wm.core.menus.install(
            event_loop.handle(),
            |wm: &mut crate::test_support::TestWm| wm.test_ctx(),
        );
        let (spec, state) = session(&wm);
        let mut command = Command::new("sh");
        command.args(["-c", r#"input=$(cat); case "$input" in *shortcut=super-c*) printf 'floating\n';; *) exit 1;; esac"#])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).process_group(0);
        menu_presenter::start(
            &mut wm.core.menus.host,
            command,
            ActiveMenu::Layout(state),
            spec.input(),
        )
        .unwrap();
        event_loop
            .dispatch(std::time::Duration::from_secs(5), &mut wm)
            .unwrap();
        assert!(!wm.core.menus.is_open());
        assert!(wm.core.menus.host.source.is_none());
        assert!(wm.core.menus.host.child.is_none());
        assert_eq!(
            wm.core
                .state
                .model
                .expect_selected_monitor()
                .current_layout_command(),
            LayoutCommand::Floating
        );
    }

    #[test]
    fn stale_context_unknown_answers_and_the_current_layout_are_noops() {
        let mut wm = wm_with_monitor();
        let monitor = wm.core.state.model.expect_selected_monitor();
        let context = MenuContext::from_monitor(monitor);
        let active = monitor.current_layout_command();
        let cases = [
            (
                MenuContext {
                    tags: TagMask::single(2).unwrap(),
                    ..context
                },
                "floating",
            ),
            (
                MenuContext {
                    monitor: MonitorId::from_raw(context.monitor.get() + 1),
                    ..context
                },
                "floating",
            ),
            (context, "arbitrary typed text"),
            (context, ""),
            (context, active.name()),
        ];
        for (context, answer) in cases {
            let mut entries = choices(active);
            apply_shortcuts(&mut entries, &[]);
            let state = LayoutMenuState { context, entries };
            wm.with_ctx(|ctx| apply(ctx, state, answer).unwrap());
            assert_eq!(
                wm.core
                    .state
                    .model
                    .expect_selected_monitor()
                    .current_layout_command(),
                active,
                "answer {answer:?} must not change the layout"
            );
        }
    }

    #[test]
    fn apply_executes_a_listed_action_for_the_unchanged_context() {
        let mut wm = wm_with_monitor();
        let monitor = wm.core.state.model.expect_selected_monitor();
        let context = MenuContext::from_monitor(monitor);
        let active = monitor.current_layout_command();
        let mut entries = choices(active);
        apply_shortcuts(&mut entries, &[]);
        wm.with_ctx(|ctx| {
            apply(ctx, LayoutMenuState { context, entries }, "floating").unwrap();
        });
        assert_eq!(
            wm.core
                .state
                .model
                .expect_selected_monitor()
                .current_layout_command(),
            LayoutCommand::Floating
        );
    }

    #[test]
    fn anchor_handles_negative_output_origins_and_reserved_side_space() {
        let monitor = crate::test_support::MonitorBuilder::new()
            .monitor_rect(crate::types::Rect::new(-1920, -200, 1920, 1080))
            .configure(|monitor| {
                monitor.available_rect = crate::types::Rect::new(-1900, -200, 1900, 1080);
                monitor.bar_height = 36;
            })
            .build();
        assert_eq!(anchor(&monitor, 140), (160, 36));
    }

    fn binding(modifiers: &str, key: &str, action: &str) -> Key {
        let mut words = action.split_whitespace();
        let name = words.next().unwrap();
        Key {
            mod_mask: modifiers.parse().unwrap(),
            keysym: crate::types::Keysym::from_name(key).unwrap(),
            action: crate::actions::KeyAction::named(
                NamedAction::parse(name, &words.map(str::to_owned).collect::<Vec<_>>()).unwrap(),
            ),
            origin: crate::types::KeybindOrigin::User,
        }
    }

    #[test]
    fn default_layout_chords_are_available_as_menu_shortcuts() {
        let bindings =
            crate::config::keybindings::default_keybinds(crate::backend::BackendKind::Wayland);
        let mut entries = choices(LayoutCommand::Tile);
        apply_shortcuts(&mut entries, &bindings);
        for (value, shortcut) in [
            ("grid", "super-c"),
            ("floating", "super-f"),
            ("maximized", "super-w"),
        ] {
            assert_eq!(
                entries
                    .iter()
                    .find(|entry| entry.value == value)
                    .unwrap()
                    .shortcut
                    .as_deref(),
                Some(shortcut),
            );
        }
    }

    #[test]
    fn shortcuts_follow_configured_bindings_and_keep_current_layout_a_noop() {
        let mut entries = choices(LayoutCommand::Grid);
        apply_shortcuts(
            &mut entries,
            &[
                binding("Super + Shift", "g", "set_layout grid"),
                binding("Super", "f", "layout_float"),
                binding("Super", "w", "toggle_tiling_maximized"),
                binding("Control", "F3", "reset_layout_to tile"),
            ],
        );
        for (value, shortcut) in [
            ("grid", "super-shift-g"),
            ("floating", "super-f"),
            ("maximized", "super-w"),
            ("reset-tile", "ctrl-f3"),
        ] {
            let entry = entries.iter().find(|entry| entry.value == value).unwrap();
            let line = entry.menu_choice().to_line();
            assert_eq!(entry.shortcut.as_deref(), Some(shortcut));
            assert!(line.contains(&format!(" shortcut={shortcut}}}")));
            assert!(line.contains(&format!(" value={value} ")));
        }
        assert_eq!(selected_action(&entries, "grid"), None);
        assert_eq!(entries[0].shortcut, None);
    }

    #[test]
    fn unsupported_shortcuts_are_omitted_without_losing_menu_metadata() {
        let mut entries = choices(LayoutCommand::Tile);
        apply_shortcuts(&mut entries, &[binding("Mod3", "g", "set_layout grid")]);
        let line = entries[1].menu_choice().to_line();
        assert!(line.starts_with("{blue icon="));
        assert!(!line.contains("shortcut="));
        assert!(line.contains(" value=grid}"));
    }

    #[test]
    fn chooser_separates_switching_from_resetting() {
        for &active in LayoutCommand::all() {
            let entries = choices(active);
            assert_eq!(selected_action(&entries, active.name()), None);
            for layout in [
                LayoutCommand::Tile,
                LayoutCommand::Grid,
                LayoutCommand::BottomStack,
            ] {
                assert_eq!(
                    selected_action(&entries, &format!("reset-{}", layout.name())),
                    Some(NamedAction::ResetLayoutTo(layout))
                );
            }
            assert_eq!(selected_action(&entries, ""), None);
            assert_eq!(selected_action(&entries, "arbitrary typed text"), None);
            for &other in LayoutCommand::all() {
                if other != active {
                    assert_eq!(
                        selected_action(&entries, other.name()),
                        Some(NamedAction::SetLayout(other))
                    );
                }
            }
        }
    }

    #[test]
    fn each_tiling_layout_has_its_reset_in_the_adjacent_column() {
        let entries = choices(LayoutCommand::Grid);
        assert_eq!(entries.len(), 8);
        for row in 0..3 {
            let Some(NamedAction::ResetLayoutTo(layout)) = entries[row + 4].action else {
                panic!("missing reset beside tiling layout");
            };
            assert_eq!(entries[row].value, layout.name());
        }
        assert_eq!(entries[3].value, "floating");
        assert_eq!(entries[7].value, "maximized");
    }

    #[test]
    fn decorated_labels_do_not_control_action_dispatch() {
        let mut entries = choices(LayoutCommand::Grid);
        for entry in &mut entries {
            entry.label = "A translated label".into();
            let line = entry.menu_choice().to_line();
            assert!(line.contains(&format!("value={}", entry.value)));
            assert!(line.contains("icon="));
        }
        assert_eq!(
            selected_action(&entries, "tile"),
            Some(NamedAction::SetLayout(LayoutCommand::Tile))
        );
        assert_eq!(selected_action(&entries, "grid"), None);
        assert_eq!(selected_action(&entries, "A translated label"), None);
    }

    #[test]
    fn chooser_uses_explicit_anchor_without_probing_the_cursor() {
        let spec = menu_spec(
            LayoutCommand::Grid,
            &MenuAnchor {
                monitor: Some(1),
                x_offset: 135,
                y_offset: 30,
            },
            &choices(LayoutCommand::Grid),
        );
        let command = spec.command();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert!(!args.contains(&"--follow-cursor"));
        for pair in [
            ["--position", "top-left"],
            ["--monitor", "1"],
            ["--x-offset", "135"],
            ["--y-offset", "30"],
        ] {
            assert!(args.windows(2).any(|args| args == pair));
        }
        assert!(args.windows(2).any(|pair| pair == ["--columns", "2"]));
        assert!(args.windows(2).any(|pair| pair == ["--lines", "4"]));
        assert!(args.windows(2).any(|pair| pair == ["--preselect", "grid"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--placeholder", "Choose window layout"])
        );
    }

    #[test]
    fn chooser_centers_when_the_bar_has_no_hit_geometry() {
        let spec = menu_spec(
            LayoutCommand::Grid,
            &MenuAnchor::default(),
            &choices(LayoutCommand::Grid),
        );
        let command = spec.command();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert!(args.windows(2).any(|pair| pair == ["--position", "center"]));
        assert!(args.windows(2).any(|pair| pair == ["--monitor", "auto"]));
        assert!(!args.contains(&"--x-offset"));
        assert!(!args.contains(&"--y-offset"));
    }
}

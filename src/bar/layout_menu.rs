//! WM-owned layout chooser. The event loop watches the presenter only while
//! it is open; its stdout callback applies the choice without per-tick polling.
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use crate::actions::{KeyAction, NamedAction};
use crate::contexts::WmCtx;
use crate::layouts::LayoutCommand;
use crate::types::{Key, Modifier, Monitor, MonitorId, TagMask};
use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken};

struct Choice {
    value: String,
    label: String,
    icon: &'static str,
    color: &'static str,
    action: Option<NamedAction>,
    shortcut: Option<String>,
}

impl Choice {
    fn line(&self) -> String {
        format!(
            "{{{} icon={} value={}{}}} {}",
            self.color,
            self.icon,
            self.value,
            self.shortcut
                .as_ref()
                .map_or_else(String::new, |key| format!(" shortcut={key}")),
            self.label
        )
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
        color: if current { "green" } else { "blue" },
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
            color: "yellow",
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

/// Translate only chords supported by instantMENU; invalid metadata would
/// otherwise cause it to treat the whole decorated row as ordinary text.
fn menu_shortcut(binding: &Key) -> Option<String> {
    let modifiers = binding.mod_mask;
    let mut shortcut = String::new();
    for modifier in modifiers.iter() {
        shortcut.push_str(match modifier {
            Modifier::Super => "super-",
            Modifier::Control => "ctrl-",
            Modifier::Alt => "alt-",
            Modifier::Shift => "shift-",
            _ => return None,
        });
    }
    let key = binding.keysym.to_string().to_lowercase();
    let key = match key.as_str() {
        "return" => "enter",
        "prior" => "page-up",
        "next" => "page-down",
        key => key,
    };
    let character =
        key.chars().count() == 1 && key.chars().all(|c| !c.is_whitespace() && !c.is_control());
    let function = key.strip_prefix('f').is_some_and(|n| {
        n.parse::<u8>()
            .is_ok_and(|n| (1..=35).contains(&n) && key == format!("f{n}"))
    });
    if !character
        && !function
        && !matches!(
            key,
            "enter"
                | "tab"
                | "space"
                | "backspace"
                | "delete"
                | "insert"
                | "left"
                | "right"
                | "up"
                | "down"
                | "home"
                | "end"
                | "page-up"
                | "page-down"
        )
    {
        return None;
    }
    if (modifiers.is_empty() && matches!(key, "enter" | "tab"))
        || (modifiers.contains(Modifier::Control)
            && !modifiers.contains(Modifier::Alt)
            && !modifiers.contains(Modifier::Super)
            && matches!(key, "c" | "g"))
    {
        return None;
    }
    shortcut.push_str(key);
    Some(shortcut)
}

fn apply_shortcuts(entries: &mut [Choice], bindings: &[Key]) {
    for entry in entries {
        let action = entry
            .action
            .clone()
            .or_else(|| LayoutCommand::from_name(&entry.value).map(NamedAction::SetLayout));
        let Some(action) = action else { continue };
        entry.shortcut = bindings
            .iter()
            .filter(|binding| {
                let KeyAction::Named(bound) = &binding.action else {
                    return false;
                };
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
            })
            .find_map(menu_shortcut);
    }
}

#[derive(Default)]
struct MenuAnchor {
    monitor: Option<u32>,
    x_offset: i32,
    y_offset: i32,
}

fn menu_command(active: LayoutCommand, anchor: &MenuAnchor) -> Command {
    let mut command = Command::new("instantmenu");
    command
        .args([
            "--position",
            if anchor.monitor.is_some() {
                "top-left"
            } else {
                "center"
            },
            "--border-width",
            "4",
            "--width",
            "auto",
            "--line-height",
            "auto",
            "--columns",
            "2",
            "--lines",
            "4",
            "--placeholder",
            "Choose window layout",
            "--insensitive",
            "--reject-no-match",
            "--preselect",
            active.name(),
        ])
        .args([
            "--monitor",
            &anchor
                .monitor
                .map_or_else(|| "auto".into(), |index| index.to_string()),
        ])
        .args(["--x-offset", &anchor.x_offset.to_string()])
        .args(["--y-offset", &anchor.y_offset.to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .process_group(0);
    command
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MenuContext {
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

type WatchOutput = dyn Fn(File, u64, MenuContext, Vec<Choice>) -> Result<RegistrationToken, String>;

#[derive(Default)]
pub(crate) struct LayoutMenuHost {
    watch: Option<Box<WatchOutput>>,
    remove: Option<Box<dyn Fn(RegistrationToken)>>,
    source: Option<RegistrationToken>,
    generation: u64,
    child: Option<Child>,
}

impl LayoutMenuHost {
    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(token) = self.source.take()
            && let Some(remove) = &self.remove
        {
            remove(token);
        }
        if let Some(mut child) = self.child.take() {
            // Keep the child unreaped until after signalling, so its pid cannot
            // be reused for an unrelated process group.
            if let Ok(pgid) = i32::try_from(child.id()) {
                crate::util::signal_process_group(pgid, libc::SIGTERM);
            } else {
                let _ = child.kill();
            }
            crate::util::reap_child_async(child);
        }
    }
}

impl Drop for LayoutMenuHost {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Backend wiring supplies only access to its shared context. Sources are
/// installed on demand and removed after a choice, dismissal, or cancellation.
pub(crate) fn install<T: 'static>(
    handle: LoopHandle<'static, T>,
    host: &mut LayoutMenuHost,
    context: for<'a> fn(&'a mut T) -> WmCtx<'a>,
) {
    let remove_handle = handle.clone();
    host.remove = Some(Box::new(move |token| remove_handle.remove(token)));
    host.watch = Some(Box::new(
        move |output, generation, menu_context, entries| {
            let mut buffer = Vec::new();
            handle
                .insert_source(
                    Generic::new(output, Interest::READ, Mode::Level),
                    move |_, output, state| {
                        let answer = match read_answer(&mut &**output, &mut buffer) {
                            Ok(None) => return Ok(PostAction::Continue),
                            Ok(Some(answer)) => answer,
                            Err(error) => {
                                log::debug!("layout chooser output: {error}");
                                String::new()
                            }
                        };
                        finish(
                            &mut context(state),
                            generation,
                            menu_context,
                            selected_action(&entries, answer.trim_end()),
                        );
                        Ok(PostAction::Remove)
                    },
                )
                .map_err(|error| format!("failed to watch layout chooser: {error}"))
        },
    ));
}

/// Read a bounded, nonblocking single-line response. Empty EOF is dismissal.
fn read_answer(output: &mut impl Read, buffer: &mut Vec<u8>) -> io::Result<Option<String>> {
    let mut chunk = [0; 128];
    loop {
        match output.read(&mut chunk) {
            Ok(0) => {
                return String::from_utf8(std::mem::take(buffer))
                    .map(Some)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
            }
            Ok(count) => {
                buffer.extend_from_slice(&chunk[..count]);
                if buffer.len() > 128 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "layout chooser response too long",
                    ));
                }
                if let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
                    return String::from_utf8(buffer[..end].to_vec())
                        .map(Some)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn anchor(monitor: &Monitor, layout_start: i32) -> (i32, i32) {
    (
        monitor.work_rect().x - monitor.monitor_rect.x + layout_start,
        monitor.bar_y() + monitor.bar_height - monitor.monitor_rect.y,
    )
}

pub(crate) fn open(ctx: &mut WmCtx<'_>) -> Result<(), String> {
    let core = ctx.core_mut();
    let monitor = core.state.model.expect_selected_monitor();
    let context = MenuContext::from_monitor(monitor);
    let active = monitor.current_layout_command();
    let placement = core
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
    let mut entries = choices(active);
    apply_shortcuts(&mut entries, &core.state.config.bindings.keys);
    start_menu(
        &mut core.bar.layout_menu,
        menu_command(active, &placement),
        context,
        entries,
    )
}

fn start_menu(
    host: &mut LayoutMenuHost,
    mut command: Command,
    context: MenuContext,
    entries: Vec<Choice>,
) -> Result<(), String> {
    if host.watch.is_none() {
        return Err("layout chooser event source is not configured".into());
    }
    let input = entries
        .iter()
        .map(Choice::line)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    host.cancel();
    let child = command
        .spawn()
        .map_err(|error| format!("failed to open layout chooser: {error}"))?;
    host.child = Some(child);
    let result = (|| {
        let child = host.child.as_mut().unwrap();
        let output = child.stdout.take().expect("piped stdout");
        let output = File::from(OwnedFd::from(output));
        // Preserve existing descriptor flags when enabling nonblocking reads.
        let fd = output.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        // Eight bounded menu rows fit in an empty pipe. Close stdin immediately
        // after writing so instantMENU can finish reading its entries.
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(input.as_bytes())
            .map_err(|error| format!("failed to populate layout chooser: {error}"))?;
        host.source = Some(host.watch.as_ref().unwrap()(
            output,
            host.generation,
            context,
            entries,
        )?);
        Ok(())
    })();
    if result.is_err() {
        host.cancel();
    }
    result
}

fn finish(ctx: &mut WmCtx<'_>, generation: u64, context: MenuContext, action: Option<NamedAction>) {
    let host = &mut ctx.core_mut().bar.layout_menu;
    if generation != host.generation {
        return;
    }
    // The callback removes its own source with PostAction::Remove.
    host.source = None;
    // A complete stdout value is the presenter's selection. Reaping is separate
    // so process shutdown can never delay the event-loop callback.
    if let Some(child) = host.child.take() {
        crate::util::reap_child_async(child);
    }
    let monitor = ctx.core().state.model.expect_selected_monitor();
    if context != MenuContext::from_monitor(monitor) {
        return;
    }
    let Some(action) = action else { return };
    if matches!(action, NamedAction::SetLayout(layout) if layout == monitor.current_layout_command())
    {
        return;
    }
    if let Err(error) = action.execute(ctx) {
        log::warn!("layout chooser: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn presenter_receives_shortcuts_and_returns_an_action_without_ipc() {
        let mut wm = wm_with_monitor();
        let mut event_loop = calloop::EventLoop::try_new().unwrap();
        install(
            event_loop.handle(),
            &mut wm.core.bar.layout_menu,
            |wm: &mut crate::test_support::TestWm| wm.test_ctx(),
        );
        let monitor = wm.core.state.model.expect_selected_monitor();
        let context = MenuContext::from_monitor(monitor);
        let active = monitor.current_layout_command();
        let mut entries = choices(active);
        apply_shortcuts(
            &mut entries,
            &crate::config::keybindings::default_keybinds(crate::backend::BackendKind::Wayland),
        );
        let mut command = Command::new("sh");
        command.args(["-c", r#"input=$(cat); case "$input" in *shortcut=super-c*) printf 'floating\n';; *) exit 1;; esac"#])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).process_group(0);
        start_menu(&mut wm.core.bar.layout_menu, command, context, entries).unwrap();
        event_loop
            .dispatch(std::time::Duration::from_secs(5), &mut wm)
            .unwrap();
        assert!(wm.core.bar.layout_menu.source.is_none());
        assert!(wm.core.bar.layout_menu.child.is_none());
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
    fn superseded_or_stale_selections_are_ignored() {
        let mut wm = wm_with_monitor();
        let monitor = wm.core.state.model.expect_selected_monitor();
        let context = MenuContext::from_monitor(monitor);
        let active = monitor.current_layout_command();
        wm.core.bar.layout_menu.cancel();
        let current_generation = wm.core.bar.layout_menu.generation;
        for (generation, context) in [
            (current_generation - 1, context),
            (
                current_generation,
                MenuContext {
                    tags: TagMask::single(2).unwrap(),
                    ..context
                },
            ),
            (
                current_generation,
                MenuContext {
                    monitor: MonitorId::from_raw(context.monitor.get() + 1),
                    ..context
                },
            ),
        ] {
            wm.with_ctx(|ctx| {
                finish(
                    ctx,
                    generation,
                    context,
                    Some(NamedAction::SetLayout(LayoutCommand::Floating)),
                )
            });
        }
        assert_eq!(
            wm.core
                .state
                .model
                .expect_selected_monitor()
                .current_layout_command(),
            active
        );
    }

    #[test]
    fn cancellation_removes_the_source_and_releases_the_child() {
        let mut wm = wm_with_monitor();
        let mut event_loop = calloop::EventLoop::try_new().unwrap();
        install(
            event_loop.handle(),
            &mut wm.core.bar.layout_menu,
            |wm: &mut crate::test_support::TestWm| wm.test_ctx(),
        );
        let monitor = wm.core.state.model.expect_selected_monitor();
        let context = MenuContext::from_monitor(monitor);
        let mut command = Command::new("sh");
        command
            .args(["-c", "exec sleep 30"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .process_group(0);
        let host = &mut wm.core.bar.layout_menu;
        start_menu(host, command, context, choices(LayoutCommand::Tile)).unwrap();
        host.cancel();
        assert!(host.source.is_none());
        assert!(host.child.is_none());
        // Removing the source also closes the pipe. No menu callback should
        // remain registered after cancellation.
        event_loop
            .dispatch(std::time::Duration::ZERO, &mut wm)
            .unwrap();
    }

    #[test]
    fn partial_output_waits_for_readiness_without_blocking() {
        let (mut output, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        output.set_nonblocking(true).unwrap();
        let mut buffer = Vec::new();
        writer.write_all(b"flo").unwrap();
        assert_eq!(read_answer(&mut output, &mut buffer).unwrap(), None);
        writer.write_all(b"ating\n").unwrap();
        assert_eq!(
            read_answer(&mut output, &mut buffer).unwrap(),
            Some("floating".into())
        );
    }

    #[test]
    fn dismissal_and_unknown_output_remove_the_source_without_changing_layout() {
        for script in ["cat >/dev/null", "cat >/dev/null; printf 'unknown\\n'"] {
            let mut wm = wm_with_monitor();
            let mut event_loop = calloop::EventLoop::try_new().unwrap();
            install(
                event_loop.handle(),
                &mut wm.core.bar.layout_menu,
                |wm: &mut crate::test_support::TestWm| wm.test_ctx(),
            );
            let monitor = wm.core.state.model.expect_selected_monitor();
            let context = MenuContext::from_monitor(monitor);
            let active = monitor.current_layout_command();
            let mut command = Command::new("sh");
            command
                .args(["-c", script])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .process_group(0);
            start_menu(
                &mut wm.core.bar.layout_menu,
                command,
                context,
                choices(active),
            )
            .unwrap();
            event_loop
                .dispatch(std::time::Duration::from_secs(5), &mut wm)
                .unwrap();
            assert!(wm.core.bar.layout_menu.source.is_none());
            assert!(wm.core.bar.layout_menu.child.is_none());
            assert_eq!(
                wm.core
                    .state
                    .model
                    .expect_selected_monitor()
                    .current_layout_command(),
                active
            );
        }
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
            action: KeyAction::named(
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
            assert_eq!(entry.shortcut.as_deref(), Some(shortcut));
            assert!(entry.line().contains(&format!(" shortcut={shortcut}}}")));
            assert!(entry.line().contains(&format!(" value={value} ")));
        }
        assert_eq!(selected_action(&entries, "grid"), None);
        assert_eq!(entries[0].shortcut, None);
    }

    #[test]
    fn unsupported_shortcuts_are_omitted_without_losing_menu_metadata() {
        for (modifiers, key) in [
            ("Mod3", "g"),
            ("Super", "Escape"),
            ("Control", "c"),
            ("", "Return"),
            ("Super", "XF86AudioMute"),
        ] {
            assert_eq!(
                menu_shortcut(&binding(modifiers, key, "set_layout grid")),
                None
            );
        }
        assert_eq!(
            menu_shortcut(&binding("Alt", "Prior", "set_layout grid")),
            Some("alt-page-up".into())
        );
        let mut entries = choices(LayoutCommand::Tile);
        apply_shortcuts(&mut entries, &[binding("Mod3", "g", "set_layout grid")]);
        assert!(entries[1].line().starts_with("{blue icon="));
        assert!(!entries[1].line().contains("shortcut="));
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
            assert!(entry.line().contains(&format!("value={}", entry.value)));
            assert!(entry.line().contains("icon="));
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
        let command = menu_command(
            LayoutCommand::Grid,
            &MenuAnchor {
                monitor: Some(1),
                x_offset: 135,
                y_offset: 30,
                ..Default::default()
            },
        );
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
}

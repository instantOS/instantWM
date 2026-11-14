//! Global MRU window chooser. instantMENU owns presentation and keyboard cycling;
//! the WM snapshots candidates and activates only a validated returned ID.
use std::collections::HashSet;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::contexts::WmCtx;
use crate::menu_presenter::{self, MenuHost};
use crate::model::WmModel;
use crate::types::WindowId;

pub(crate) type WindowMenuHost = MenuHost<Vec<WindowId>>;

/// Current window first, then committed focus history across all tags/outputs.
/// Unvisited clients follow in stable ID order. Hidden scratchpads and clients
/// that decline keyboard focus are not activation candidates.
fn candidates(model: &WmModel) -> Vec<WindowId> {
    let mut unvisited: Vec<_> = model.clients_iter_all().map(|(_, c)| c.win).collect();
    unvisited.sort_by_key(|win| win.0);
    let mut seen = HashSet::new();
    model
        .selected_win()
        .into_iter()
        .chain(model.focus_history.iter().copied())
        .chain(unvisited)
        .filter(|win| seen.insert(*win))
        .filter(|win| {
            model.client(*win).is_some_and(|c| {
                !c.never_focus && (!c.is_scratchpad() || c.is_scratchpad_visible())
            })
        })
        .collect()
}

fn menu_command(windows: &[WindowId], selected: Option<WindowId>, reverse: bool) -> Command {
    let mut command = Command::new("instantmenu");
    command.args([
        "--lines",
        "40",
        "--alt-tab",
        "--position",
        "center",
        "--border-width",
        "4",
        "--width",
        "700",
        "--insensitive",
        "--reject-no-match",
        "--prompt",
        "windows",
    ]);
    // The triggering Tab was consumed by the WM, before the menu existed.
    // Account for that first step here; subsequent Tab presses go to the menu.
    let index = if reverse {
        windows.len().saturating_sub(1)
    } else {
        usize::from(windows.len() > 1 && windows.first().copied() == selected)
    };
    if let Some(win) = windows.get(index) {
        command.args(["--preselect", &win.0.to_string()]);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .process_group(0);
    command
}

pub(crate) fn open(ctx: &mut WmCtx<'_>, reverse: bool) -> Result<(), String> {
    let core = ctx.core_mut();
    // Repeated triggers before the presenter acquires the keyboard must not
    // spawn competing menus or discard the first session's result.
    if core.window_menu.child.is_some() {
        return Ok(());
    }
    let model = &core.state.model;
    let windows = candidates(model);
    if windows.is_empty() {
        return Ok(());
    }
    let selected = model.selected_win();
    let input = windows
        .iter()
        .map(|win| {
            let client = model
                .client(*win)
                .expect("snapshot contains managed clients");
            // Titles are untrusted text: prevent newlines from injecting menu rows.
            let title: String = client
                .name
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            let icon = if selected == Some(*win) {
                "codwindowactive"
            } else {
                "codwindow"
            };
            format!("{{value={} icon={icon}}} {title}\n", win.0)
        })
        .collect::<String>();
    let command = menu_command(&windows, selected, reverse);
    menu_presenter::start(&mut core.window_menu, command, windows, input)
}

pub(crate) fn install<T: 'static>(
    handle: calloop::LoopHandle<'static, T>,
    host: &mut WindowMenuHost,
    context: for<'a> fn(&'a mut T) -> WmCtx<'a>,
) {
    menu_presenter::install(handle, host, context, finish);
}

fn finish(ctx: &mut WmCtx<'_>, generation: u64, windows: Vec<WindowId>, answer: &str) {
    let host = &mut ctx.core_mut().window_menu;
    if host.generation != generation {
        return;
    }
    host.source = None;
    if let Some(child) = host.child.take() {
        crate::util::reap_child_async(child);
    }
    let Some(win) = windows.into_iter().find(|win| win.0.to_string() == answer) else {
        return;
    };
    let Some(client) = ctx.core().state.model.client(win) else {
        return;
    };
    if client.never_focus || (client.is_scratchpad() && !client.is_scratchpad_visible()) {
        return;
    }
    if client.is_hidden {
        crate::client::show_window(ctx, win);
    }
    crate::focus::activate_client(ctx, win);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{MonitorBuilder, TestWm, add_client};
    use crate::types::{Client, TagMask};

    fn fixture() -> TestWm {
        let mut wm = TestWm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.model.tags.num_tags = 2;
        for _ in 0..2 {
            let monitor = wm.core.state.model.monitors.push(
                MonitorBuilder::new()
                    .tag_count(2)
                    .selected_tags(TagMask::single(1).unwrap())
                    .build(),
            );
            for offset in 0..2 {
                let id = wm.core.state.model.client_count() as u32 + 1;
                add_client(
                    &mut wm.core.state.model,
                    monitor,
                    Client {
                        win: WindowId(id),
                        name: format!("Window {id}"),
                        tags: TagMask::single(offset + 1).unwrap(),
                        ..Client::default()
                    },
                );
            }
        }
        let first = wm.core.state.model.monitors.iter().next().unwrap().0;
        wm.core.state.model.monitors.set_selected(first);
        wm
    }

    fn activate(wm: &mut TestWm, id: u32) {
        wm.with_ctx(|ctx| assert!(crate::focus::activate_client(ctx, WindowId(id))));
    }

    #[test]
    fn switching_across_tags_and_outputs_preserves_mru_and_toggles_back() {
        let mut wm = fixture();
        for id in [3, 1, 2, 4] {
            activate(&mut wm, id);
        }
        assert_eq!(candidates(&wm.core.state.model), [4, 2, 1, 3].map(WindowId));
        // The first candidate after the current window is the previous focus.
        activate(&mut wm, 2);
        assert_eq!(candidates(&wm.core.state.model), [2, 4, 1, 3].map(WindowId));
        activate(&mut wm, 4);
        assert_eq!(candidates(&wm.core.state.model), [4, 2, 1, 3].map(WindowId));
        wm.core.state.model.remove_client(WindowId(2));
        assert!(!wm.core.state.model.focus_history.contains(&WindowId(2)));
        assert_eq!(candidates(&wm.core.state.model), [4, 1, 3].map(WindowId));
    }

    #[test]
    fn presenter_starts_on_previous_window_or_last_for_reverse() {
        let windows = [WindowId(8), WindowId(5), WindowId(2)];
        for (reverse, expected) in [(false, "5"), (true, "2")] {
            let command = menu_command(&windows, Some(windows[0]), reverse);
            let args: Vec<_> = command.get_args().map(|s| s.to_str().unwrap()).collect();
            assert!(
                args.windows(2)
                    .any(|pair| pair == ["--preselect", expected])
            );
        }
        let command = menu_command(&windows[..1], Some(windows[0]), false);
        let args: Vec<_> = command.get_args().map(|s| s.to_str().unwrap()).collect();
        assert!(args.windows(2).any(|pair| pair == ["--preselect", "8"]));
    }

    #[test]
    fn unvisited_windows_are_stable_and_non_focusable_clients_are_skipped() {
        let mut wm = fixture();
        wm.core
            .state
            .model
            .client_mut(WindowId(2))
            .unwrap()
            .never_focus = true;
        assert_eq!(candidates(&wm.core.state.model), [1, 3, 4].map(WindowId));
        // With no current window the initial forward step starts at the MRU
        // entry, rather than skipping it as though it were already focused.
        let command = menu_command(&[WindowId(3), WindowId(1)], None, false);
        let args: Vec<_> = command.get_args().map(|s| s.to_str().unwrap()).collect();
        assert!(args.windows(2).any(|pair| pair == ["--preselect", "3"]));
    }

    #[test]
    fn invalid_dismissed_stale_and_closed_choices_do_not_change_focus() {
        let mut wm = fixture();
        activate(&mut wm, 1);
        let generation = wm.core.window_menu.generation;
        for answer in ["", "Window 2", "999", "2\n1"] {
            wm.with_ctx(|ctx| finish(ctx, generation, vec![WindowId(2)], answer));
            assert_eq!(wm.core.state.model.selected_win(), Some(WindowId(1)));
        }
        wm.with_ctx(|ctx| finish(ctx, generation.wrapping_sub(1), vec![WindowId(2)], "2"));
        wm.core.state.model.remove_client(WindowId(2));
        wm.with_ctx(|ctx| finish(ctx, generation, vec![WindowId(2)], "2"));
        assert_eq!(wm.core.state.model.selected_win(), Some(WindowId(1)));
    }

    #[test]
    fn event_loop_activates_only_the_presented_id_and_restores_minimized_windows() {
        let mut wm = fixture();
        activate(&mut wm, 1);
        wm.core
            .state
            .model
            .client_mut(WindowId(2))
            .unwrap()
            .is_hidden = true;
        let mut event_loop = calloop::EventLoop::try_new().unwrap();
        install(
            event_loop.handle(),
            &mut wm.core.window_menu,
            |wm: &mut TestWm| wm.test_ctx(),
        );
        let mut command = Command::new("sh");
        command
            .args(["-c", "cat >/dev/null; printf '2\\n'"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .process_group(0);
        menu_presenter::start(
            &mut wm.core.window_menu,
            command,
            vec![WindowId(1), WindowId(2)],
            "rows\n".into(),
        )
        .unwrap();
        event_loop
            .dispatch(std::time::Duration::from_secs(5), &mut wm)
            .unwrap();
        assert_eq!(wm.core.state.model.selected_win(), Some(WindowId(2)));
        assert!(!wm.core.state.model.client(WindowId(2)).unwrap().is_hidden);
        assert!(wm.core.window_menu.child.is_none());
        assert!(wm.core.window_menu.source.is_none());
    }
}

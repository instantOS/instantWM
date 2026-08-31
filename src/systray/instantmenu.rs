//! instantMENU-backed presentation of a hosted DBus tray menu.
//!
//! When [`crate::core_state::TrayMenuBackend`] resolves to instantMENU, the
//! bar delegates the open tray-menu session to an `instantmenu` process: the
//! current level's entries are written to its stdin as icon-annotated lines,
//! and its single-line stdout answer maps back to the entry's
//! [`MenuAction`]. Dismissal (Escape or an outside press) closes the session
//! exactly like a press outside the bar-native menu.
//!
//! The process runs in its own process group with piped stdio, mirroring the
//! status-command lifecycle: a reader thread owns the child, publishes the
//! outcome through a channel, and pings the event loop so the next tick
//! reconciles state deterministically.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use calloop::ping::Ping;

use crate::core_state::TrayMenuBackend;
#[allow(unused_imports)]
use crate::systray::{MenuAction, MenuEntry, MenuToggle, MenuView};
use crate::wm::Wm;

const INSTANTMENU_BIN: &str = "instantmenu";
/// Fixed context-menu width. `auto` would apply instantMENU's launcher
/// minimum (500 px), far wider than a tray menu needs.
const MENU_WIDTH: i32 = 340;

/// Final result of one instantmenu process, delivered by its reader thread.
enum MenuOutcome {
    /// An item was chosen; `label` is the `value` instantMENU printed
    /// (the entry index as string), or legacy display label for fallback.
    Selected { session_id: u64, label: String },
    /// The menu was dismissed without a choice (Escape or an outside press).
    Dismissed { session_id: u64 },
}

// Availability of the binary is scanned from $PATH once and cached; a failed
// spawn also demotes it so `auto` (and a forced-but-missing `instantmenu`)
// falls back to bar-native rendering instead of retrying every tick.
const AVAILABLE: u8 = 1;
const UNAVAILABLE: u8 = 2;
static AVAILABILITY: AtomicU8 = AtomicU8::new(0);

fn instantmenu_available() -> bool {
    match AVAILABILITY.load(Ordering::Relaxed) {
        AVAILABLE => true,
        UNAVAILABLE => false,
        _ => {
            let found = std::env::var_os("PATH")
                .map(|path| {
                    std::env::split_paths(&path).any(|dir| dir.join(INSTANTMENU_BIN).is_file())
                })
                .unwrap_or(false);
            AVAILABILITY.store(
                if found { AVAILABLE } else { UNAVAILABLE },
                Ordering::Relaxed,
            );
            found
        }
    }
}

fn reset_availability() {
    AVAILABILITY.store(0, Ordering::Relaxed);
}

/// State of the external instantmenu presentation, owned by the bar.
pub(crate) struct InstantMenuHost {
    sender: Sender<MenuOutcome>,
    outcomes: Receiver<MenuOutcome>,
    /// Event-loop wake shared with the reader threads.
    wake: Option<Ping>,
    /// Process-group id of the live instantmenu, if any. The reader thread
    /// clears its own entry after reaping, so a stale pid is never signalable.
    child_pgid: Arc<Mutex<Option<i32>>>,
    /// Session and view currently handed to instantmenu. Compared against
    /// the presented menu to decide whether to (re)spawn.
    presented: Option<(u64, MenuView)>,
    /// Last observed config value; a change rescans binary availability.
    last_backend: TrayMenuBackend,
}

impl Default for InstantMenuHost {
    fn default() -> Self {
        Self::new()
    }
}

impl InstantMenuHost {
    pub(crate) fn new() -> Self {
        let (sender, outcomes) = channel();
        Self {
            sender,
            outcomes,
            wake: None,
            child_pgid: Arc::new(Mutex::new(None)),
            presented: None,
            last_backend: TrayMenuBackend::Auto,
        }
    }

    pub(crate) fn set_wake(&mut self, wake: Option<Ping>) {
        self.wake = wake;
    }

    /// Whether the external instantmenu presents the tray menu, i.e. the bar
    /// must not render the menu overlay itself.
    pub(crate) fn hosting(&self, backend: TrayMenuBackend) -> bool {
        backend != TrayMenuBackend::StatusBar && instantmenu_available()
    }
}

impl Drop for InstantMenuHost {
    fn drop(&mut self) {
        // Teardown at WM shutdown; backend death also closes the menu, but
        // explicit termination avoids a flash on nested backends.
        kill_child(&self.child_pgid);
    }
}

/// Reconcile the external instantmenu with the presented tray menu.
///
/// Called once per tick after [`Wm::poll_systray`]: drains finished processes
/// (a selection may close the session or navigate a level), then spawns,
/// replaces, or tears down the child to match the session state. Returns
/// `true` when bar-visible content changed.
pub(crate) fn drive_instantmenu_menu(wm: &mut Wm) -> bool {
    let backend = wm.core.config.systray.menu_backend;
    {
        let host = &mut wm.bar.systray_host.instantmenu;
        if backend != host.last_backend {
            host.last_backend = backend;
            reset_availability();
        }
    }
    let hosting = wm.bar.systray_host.instantmenu.hosting(backend);

    let mut changed = false;
    // Outcomes are drained before reconciliation so a selection can close or
    // navigate the session within this same tick.
    let outcomes: Vec<MenuOutcome> = wm
        .bar
        .systray_host
        .instantmenu
        .outcomes
        .try_iter()
        .collect();
    for outcome in outcomes {
        match outcome {
            MenuOutcome::Selected { session_id, label } => {
                changed |= handle_selection(wm, session_id, &label, hosting);
            }
            MenuOutcome::Dismissed { session_id } => {
                if session_is_open(wm, session_id) {
                    close_session(wm);
                    changed |= !hosting;
                }
            }
        }
    }

    let host = &mut wm.bar.systray_host.instantmenu;
    let presentation = wm.bar.systray_host.menu.presentation();
    let wanted = if hosting { presentation.as_ref() } else { None };

    match wanted {
        None => {
            let was_presented = host.presented.take().is_some();
            kill_child(&host.child_pgid);
            changed |= was_presented && !hosting;
        }
        Some(presentation) => {
            let fingerprint = (presentation.session_id, presentation.view.clone());
            if host.presented.as_ref() != Some(&fingerprint) {
                kill_child(&host.child_pgid);
                let lines = build_lines(&presentation.view);
                if lines.is_empty() {
                    // Nothing selectable at all: an empty launcher window
                    // would flash and strand the session, so close it.
                    host.presented = None;
                    close_session(wm);
                    changed = true;
                } else if spawn_menu(
                    host,
                    presentation.session_id,
                    lines,
                    wm.core.derived.bar_height,
                ) {
                    host.presented = Some(fingerprint);
                    changed = true;
                } else {
                    // Spawn failed and availability is demoted: hosting is
                    // already false again, so this same tick falls through to
                    // bar-native rendering. Leave `presented` unset.
                    host.presented = None;
                }
            }
        }
    }
    changed
}

/// Map a chosen instantmenu line back to its entry and dispatch the action.
///
/// With the `value=<index>` integration, instantMENU outputs the `value`
/// attribute (the entry's index in `MenuView`) rather than the display label.
/// This decouples selection from fragile label equality and handles duplicate
/// labels. A failing parse falls back to legacy label matching for backward
/// compatibility.
fn handle_selection(wm: &mut Wm, session_id: u64, label: &str, hosting: bool) -> bool {
    let Some(presentation) = wm.bar.systray_host.menu.presentation() else {
        return false;
    };
    if presentation.session_id != session_id {
        return false;
    }
    // Prefer index/value parsing; fall back to legacy label comparison.
    let entry = if let Ok(index) = label.parse::<usize>() {
        match presentation.view.entries.get(index) {
            Some(entry) => entry,
            None => {
                // Unknown index: treat as dismissal.
                close_session(wm);
                return !hosting;
            }
        }
    } else if let Some(entry) = presentation
        .view
        .entries
        .iter()
        .find(|entry| entry.display_label() == label)
    {
        entry
    } else {
        // Unknown output text: treat the exchange as a dismissal.
        close_session(wm);
        return !hosting;
    };
    if entry.separator || !entry.enabled {
        // Parity with the bar-native menu, where a disabled entry consumes
        // the press but stays inert: redisplay the same level.
        wm.bar.systray_host.instantmenu.presented = None;
        return false;
    }
    let Some(runtime) = wm.bar.systray_host.runtime.as_ref() else {
        return false;
    };
    runtime.dispatch_menu_action(session_id, entry.action);
    // Activate closes through the worker's MenuChanged update; submenu
    // navigation replaces the view and respawns on a later tick.
    false
}

fn session_is_open(wm: &Wm, session_id: u64) -> bool {
    wm.bar
        .systray_host
        .menu
        .presentation()
        .is_some_and(|presentation| presentation.session_id == session_id)
}

/// Close the open session on the main thread and tell the worker, mirroring
/// [`crate::systray::close_menu`].
fn close_session(wm: &mut Wm) {
    let Some(session_id) = wm.bar.systray_host.menu.close() else {
        return;
    };
    if let Some(runtime) = wm.bar.systray_host.runtime.as_ref() {
        runtime.close_menu(session_id);
    }
}

fn instantmenu_command(count: i32, bar_height: i32) -> Command {
    let mut command = Command::new(INSTANTMENU_BIN);
    command
        .arg("--position")
        .arg("top-right")
        .arg("--border-width")
        .arg("3")
        .arg("--lines")
        .arg(count.to_string())
        .arg("--width")
        .arg(MENU_WIDTH.to_string())
        // Open directly below a top bar, where the bar-native menu lives.
        .arg("--y-offset")
        .arg(bar_height.max(0).to_string())
        // Follow keyboard focus: the tray menu opens on the selected output.
        .arg("--monitor")
        .arg("auto")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .process_group(0);
    command
}

/// Spawn an instantmenu for `session_id` showing one line per selectable
/// entry. The reader thread publishes the outcome and pings the loop.
fn spawn_menu(
    host: &mut InstantMenuHost,
    session_id: u64,
    lines: Vec<String>,
    bar_height: i32,
) -> bool {
    let Ok(count) = i32::try_from(lines.len()) else {
        return false;
    };
    let mut items = lines.join("\n");
    items.push('\n');

    let mut command = instantmenu_command(count, bar_height);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            log::warn!("systray menu: failed to spawn instantmenu: {error}");
            AVAILABILITY.store(UNAVAILABLE, Ordering::Relaxed);
            return false;
        }
    };

    let Ok(pgid) = i32::try_from(child.id()) else {
        let _ = child.kill();
        return false;
    };
    if let Ok(mut slot) = host.child_pgid.lock() {
        *slot = Some(pgid);
    }

    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let sender = host.sender.clone();
    let wake = host.wake.clone();
    let pgid_slot = Arc::clone(&host.child_pgid);

    let reader = std::thread::Builder::new()
        .name("instantwm-tray-menu".to_owned())
        .spawn(move || {
            let outcome = match run_child(&mut child, stdin, stdout, &items) {
                Some(label) => MenuOutcome::Selected { session_id, label },
                None => MenuOutcome::Dismissed { session_id },
            };
            // Drop our pgid before publishing: once wait() has reaped the
            // child the pid may be recycled and must not stay signalable.
            if let Ok(mut slot) = pgid_slot.lock()
                && slot.is_some_and(|pid| pid == pgid)
            {
                *slot = None;
            }
            let _ = sender.send(outcome);
            if let Some(wake) = wake.as_ref() {
                wake.ping();
            }
        });
    if reader.is_err() {
        log::warn!("systray menu: failed to spawn instantmenu reader thread");
        kill_child(&host.child_pgid);
        return false;
    }
    true
}

/// Feed the item lines, then read instantMENU's single-line answer. `None`
/// means no selection (empty output, or a failed/exited process).
fn run_child(
    child: &mut Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: Option<std::process::ChildStdout>,
    items: &str,
) -> Option<String> {
    if let Some(mut stdin) = stdin {
        let _ = stdin.write_all(items.as_bytes());
        let _ = stdin.flush();
    } // dropped here: EOF tells instantmenu the item stream is complete
    let label = stdout
        .and_then(|stdout| BufReader::new(stdout).lines().next())
        .and_then(|line| line.ok())
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty());
    let _ = child.wait();
    label
}

/// Terminate the live instantmenu process group, if any.
fn kill_child(pgid_slot: &Arc<Mutex<Option<i32>>>) {
    if let Ok(mut slot) = pgid_slot.lock()
        && let Some(pgid) = slot.take()
    {
        // TERM lets instantmenu release its surfaces; a missing group (ESRCH)
        // means the process is already gone, which is the goal.
        unsafe {
            libc::kill(-pgid, libc::SIGTERM);
        }
    }
}

/// One instantMENU input line per selectable entry; separators have no
/// equivalent in a launcher list and are skipped. Each line carries
/// `value=<index>` where index is the entry's position in `MenuView`, so
/// instantMENU's output is the stable index rather than the display label.
fn build_lines(view: &MenuView) -> Vec<String> {
    view.entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry_line(index, entry))
        .collect()
}

fn entry_line(index: usize, entry: &MenuEntry) -> Option<String> {
    if entry.separator {
        return None;
    }
    let mut attrs = Vec::new();
    if !entry.enabled {
        attrs.push("fade".to_string());
    }
    if let Some(icon) = icon_for_entry(entry) {
        attrs.push(format!("icon={icon}"));
    }
    // Stable unique output for robust selection (duplicate labels, markup chars).
    // Index is integer, so no quoting needed; kept as bare value.
    attrs.push(format!("value={index}"));
    let mut line = String::new();
    if !attrs.is_empty() {
        line.push('{');
        line.push_str(&attrs.join(" "));
        line.push_str("} ");
    }
    line.push_str(&entry.display_label());
    Some(line)
}

/// Keyword-to-icon mapping for entry labels, consulted in order. Every name
/// must exist in instantMENU's catalog: an unknown `icon=` value would make
/// instantMENU render the whole attribute block as literal text. Substring
/// hazards drive the order — e.g. "restart" must win over "start", and
/// "display" over "play" ("display" contains "play").
const LABEL_ICONS: &[(&str, &str)] = &[
    ("restart", "restart"),
    ("reboot", "restart"),
    ("display", "monitor"),
    ("screen", "monitor"),
    ("monitor", "monitor"),
    ("settings", "settings"),
    ("preferences", "settings"),
    ("options", "settings"),
    ("configure", "settings"),
    ("check for updates", "refresh"),
    ("update", "refresh"),
    ("refresh", "refresh"),
    ("reload", "refresh"),
    ("sync", "refresh"),
    ("documentation", "book"),
    ("manual", "book"),
    ("help", "help-circle"),
    ("support", "help-circle"),
    ("about", "information"),
    ("sign out", "logout"),
    ("logout", "logout"),
    ("log off", "logout"),
    ("quit", "logout"),
    ("exit", "exit-to-app"),
    ("shut down", "poweroff"),
    ("shutdown", "poweroff"),
    ("power off", "poweroff"),
    ("power", "poweroff"),
    ("suspend", "power-sleep"),
    ("sleep", "power-sleep"),
    ("lock", "lock"),
    ("new", "plus"),
    ("create", "plus"),
    ("add", "plus"),
    ("open", "open-in-new"),
    ("launch", "open-in-new"),
    ("notifications", "bell"),
    ("mute", "mute"),
    ("unmute", "volume"),
    ("volume", "volume"),
    ("audio", "volume"),
    ("sound", "volume"),
    ("pause", "pause"),
    ("play", "play"),
    ("resume", "play"),
    ("start", "play"),
    ("enable", "toggle-switch"),
    ("disable", "toggle-switch"),
    ("toggle", "toggle-switch"),
    ("close", "window-close"),
    ("hide", "window-close"),
    ("keyboard", "keyboard"),
    ("network", "wifi"),
    ("wifi", "wifi"),
    ("wlan", "wifi"),
    ("bluetooth", "bluetooth"),
    ("battery", "battery"),
    ("folder", "folder"),
    ("directory", "folder"),
    ("file", "file"),
];

/// Icon for an entry: structural entries are fixed, otherwise the first
/// keyword hit in [`LABEL_ICONS`] wins.
fn icon_for_entry(entry: &MenuEntry) -> Option<&'static str> {
    match entry.action {
        MenuAction::Back => return Some("arrow-left"),
        MenuAction::OpenSubmenu(_) => return Some("chevron-right"),
        MenuAction::Activate(_) => {}
    }
    let label = entry.label.to_lowercase();
    LABEL_ICONS
        .iter()
        .find_map(|(needle, icon)| label.contains(needle).then_some(*icon))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Names verified against instantMENU's generated icon catalog
    /// (`src/icons/names.bin`, normalized to lowercase alphanumerics).
    const CATALOG: &[&str] = &[
        "arrowleft",
        "battery",
        "bell",
        "bluetooth",
        "book",
        "check",
        "chevronright",
        "circle",
        "close",
        "exittoapp",
        "file",
        "folder",
        "helpcircle",
        "information",
        "keyboard",
        "lock",
        "logout",
        "monitor",
        "mute",
        "openinnew",
        "pause",
        "play",
        "plus",
        "power",
        "poweroff",
        "powersleep",
        "refresh",
        "restart",
        "settings",
        "toggleswitch",
        "volume",
        "wifi",
        "windowclose",
    ];

    fn normalized(name: &str) -> String {
        name.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    }

    fn entry(label: &str) -> MenuEntry {
        MenuEntry {
            label: label.to_string(),
            width: label.chars().count() as i32 * 8 + 20,
            enabled: true,
            separator: false,
            toggle: MenuToggle::None,
            action: MenuAction::Activate(0),
        }
    }

    #[test]
    fn every_mapped_icon_exists_in_the_catalog() {
        assert!(normalized("arrow-left") == "arrowleft");
        for (_, icon) in LABEL_ICONS {
            assert!(
                CATALOG.contains(&normalized(icon).as_str()),
                "icon {icon:?} is not in the verified catalog"
            );
        }
        for icon in ["arrow-left", "chevron-right"] {
            assert!(CATALOG.contains(&normalized(icon).as_str()));
        }
    }

    #[test]
    fn keyword_order_resolves_substring_hazards() {
        assert_eq!(icon_for_entry(&entry("Restart now")), Some("restart"));
        assert_eq!(icon_for_entry(&entry("Display settings")), Some("monitor"));
        assert_eq!(icon_for_entry(&entry("Play file")), Some("play"));
        // "profile" contains "file"; the file icon is the accepted fallback.
        assert_eq!(icon_for_entry(&entry("Profile")), Some("file"));
    }

    #[test]
    fn structural_entries_get_fixed_icons() {
        let mut back = entry("‹ Back");
        back.action = MenuAction::Back;
        let mut submenu = entry("Preferences");
        submenu.action = MenuAction::OpenSubmenu(7);

        assert_eq!(icon_for_entry(&back), Some("arrow-left"));
        assert_eq!(icon_for_entry(&submenu), Some("chevron-right"));
    }

    #[test]
    fn display_label_composes_toggle_and_submenu_markers() {
        let mut checked = entry("Enabled");
        checked.toggle = MenuToggle::Check(true);
        let mut submenu = entry("Preferences");
        submenu.action = MenuAction::OpenSubmenu(3);

        assert_eq!(checked.display_label(), "✓ Enabled");
        assert_eq!(submenu.display_label(), "Preferences ›");
    }

    #[test]
    fn entry_lines_are_valid_markup_and_round_trip_to_display_labels() {
        let mut disabled = entry("Quit");
        disabled.enabled = false;
        let mut separator = entry("");
        separator.separator = true;
        let view = MenuView {
            entries: vec![entry("Open"), disabled, separator, entry("Settings")],
        };

        let lines = build_lines(&view);
        assert_eq!(lines.len(), 3, "separators produce no line");

        let expected = [
            "{icon=open-in-new value=0} Open".to_string(),
            "{fade icon=logout value=1} Quit".to_string(),
            "{icon=settings value=3} Settings".to_string(),
        ];
        assert_eq!(lines, expected);

        // Each line's payload after the attribute block is still the display label,
        // and the `value` attribute round-trips to the entry index.
        for line in &lines {
            let (attrs, payload) = line.split_once("} ").expect("attribute block");
            // attrs includes leading '{'
            let attrs = &attrs[1..];
            let value_str = attrs
                .split_whitespace()
                .find_map(|attr| attr.strip_prefix("value="))
                .expect("value attr present");
            // Quote handling: instantMENU may quote values containing spaces, strip quotes.
            let value_str = value_str.trim_matches('"');
            let index: usize = value_str.parse().expect("value is integer index");
            let entry = &view.entries[index];
            assert!(!entry.separator, "value must not point to separator");
            assert_eq!(payload, entry.display_label());
            // Also verify that parsing the instantMENU output (value) maps back correctly.
            let parsed: usize = value_str.parse().unwrap();
            assert_eq!(view.entries[parsed].label, entry.label);
        }

        // Duplicate labels get distinct values and round-trip correctly.
        let dup_view = MenuView {
            entries: vec![entry("Same"), entry("Same")],
        };
        let dup_lines = build_lines(&dup_view);
        assert_eq!(dup_lines.len(), 2);
        assert!(dup_lines[0].contains("value=0"));
        assert!(dup_lines[1].contains("value=1"));
        assert_ne!(dup_lines[0], dup_lines[1]);
        for line in &dup_lines {
            let (attrs, _) = line.split_once("} ").unwrap();
            let attrs = &attrs[1..];
            let v = attrs
                .split_whitespace()
                .find_map(|a| a.strip_prefix("value="))
                .unwrap()
                .trim_matches('"');
            let idx: usize = v.parse().unwrap();
            assert_eq!(dup_view.entries[idx].label, "Same");
        }
    }

    #[test]
    fn an_all_separator_level_has_no_lines() {
        let mut separator = entry("");
        separator.separator = true;
        let view = MenuView {
            entries: vec![separator],
        };
        assert!(build_lines(&view).is_empty());
    }

    #[test]
    fn hosting_depends_on_backend_and_availability() {
        reset_availability();
        // The cached scan result is environment-dependent; only assert the
        // backend dimension, which is not.
        let host = InstantMenuHost::new();
        assert!(!host.hosting(TrayMenuBackend::StatusBar));
    }

    #[test]
    fn command_arguments_have_valid_structure_and_numeric_values() {
        let count = 5;
        let bar_height = 32;
        let cmd = instantmenu_command(count, bar_height);
        let args: Vec<&str> = cmd.get_args().map(|s| s.to_str().unwrap()).collect();

        // Ensure arguments are structured as valid (flag, value) pairs
        assert!(
            args.len().is_multiple_of(2),
            "arguments should be key-value pairs"
        );

        let mut map = std::collections::HashMap::new();
        for chunk in args.as_chunks::<2>().0 {
            let flag = chunk[0];
            let val = chunk[1];
            assert!(flag.starts_with("--"), "flag {flag} must start with '--'");
            assert!(
                !val.starts_with("--"),
                "option value {val:?} for {flag} must not start with '--' (malformed flag)"
            );
            map.insert(flag, val);
        }

        // Validate numeric flags parse as numbers
        assert_eq!(
            map.get("--lines").and_then(|v| v.parse::<i32>().ok()),
            Some(count)
        );
        assert_eq!(
            map.get("--y-offset").and_then(|v| v.parse::<i32>().ok()),
            Some(bar_height)
        );
        assert!(
            map.get("--border-width")
                .and_then(|v| v.parse::<i32>().ok())
                .is_some()
        );
        assert!(
            map.get("--width")
                .and_then(|v| v.parse::<i32>().ok())
                .is_some()
        );
        assert_eq!(map.get("--position"), Some(&"top-right"));
    }
}

//! Event-driven instantMENU presentation of hosted D-Bus tray menus.
//! D-Bus sessions and submenu navigation stay in the tray worker. Each process
//! receives an immutable presentation snapshot; replacements invalidate its
//! generation so late output cannot select from, or dismiss, a newer menu.
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::contexts::WmCtx;
use crate::core_state::{TrayMenuBackend, WmCore};
use crate::menu_presenter::{self, MenuHost};
use crate::systray::{MenuAction, MenuEntry, MenuView, TrayMenuPresentation};

const INSTANTMENU_BIN: &str = "instantmenu";
const MENU_WIDTH: i32 = 340;

fn instantmenu_on_path() -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(INSTANTMENU_BIN).is_file()))
        .unwrap_or(false)
}

#[derive(Default)]
pub(crate) struct InstantMenuHost {
    presenter: MenuHost<TrayMenuPresentation>,
    /// The view revision and placement already submitted, including while a
    /// selection is awaiting its asynchronous D-Bus response.
    presented: Option<(u64, i32)>,
    last_backend: TrayMenuBackend,
    available: Option<bool>,
}

impl InstantMenuHost {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn hosting(&self, backend: TrayMenuBackend) -> bool {
        backend != TrayMenuBackend::StatusBar && self.available == Some(true)
    }

    fn refresh_availability(&mut self, backend: TrayMenuBackend) {
        if backend != self.last_backend {
            self.last_backend = backend;
            self.available = None;
        }
        if backend != TrayMenuBackend::StatusBar && self.available.is_none() {
            self.available = Some(instantmenu_on_path());
        }
    }
}

pub(crate) fn install<T: 'static>(
    handle: calloop::LoopHandle<'static, T>,
    host: &mut crate::systray::SystrayHost,
    context: for<'a> fn(&'a mut T) -> WmCtx<'a>,
) {
    menu_presenter::install(handle, &mut host.instantmenu.presenter, context, finish);
    host.menu.invalidate_presentation();
}

/// Called only when the tray view or bar presentation changes. There is no
/// per-tick channel drain, menu cloning, process-ID lock, or child polling.
pub(crate) fn reconcile(wm: &mut WmCore) -> bool {
    let host = &mut wm.bar.systray_host.instantmenu;
    // X11 builds its first bars before installing event-loop sources.
    if !host.presenter.is_installed() {
        return false;
    }
    let backend = wm.state.config.systray.menu_backend;
    let was_hosting = host.hosting(backend);
    host.refresh_availability(backend);
    let hosting = host.hosting(backend);
    let revision = wm.bar.systray_host.menu.revision();
    let height = wm.state.config.bar_metrics().height;
    let host = &mut wm.bar.systray_host.instantmenu;
    if !hosting || !wm.bar.systray_host.menu.is_presented() {
        let was_presented = host.presented.take().is_some();
        host.presenter.cancel();
        return was_presented || was_hosting != hosting;
    }
    if host.presented == Some((revision, height)) {
        return false;
    }
    let presentation = wm
        .bar
        .systray_host
        .menu
        .presentation()
        .expect("open tray view");
    let lines = build_lines(&presentation.view);
    if lines.is_empty() {
        host.presenter.cancel();
        host.presented = None;
        close_session(wm);
        return true;
    }
    let command = instantmenu_command(lines.len().try_into().unwrap_or(i32::MAX), height);
    if start_menu(host, command, presentation, lines).is_err() {
        // A failed presenter falls back to bar-native rendering. Retry only
        // after the backend setting changes, rather than on every frame.
        host.available = Some(false);
        return true;
    }
    host.presented = Some((revision, height));
    true
}

fn start_menu(
    host: &mut InstantMenuHost,
    command: Command,
    presentation: TrayMenuPresentation,
    lines: Vec<String>,
) -> Result<(), String> {
    host.presented = None;
    let input = lines.join("\n") + "\n";
    menu_presenter::start(&mut host.presenter, command, presentation, input).inspect_err(|error| {
        log::warn!("systray menu: {error}");
    })
}

fn selected_entry<'a>(view: &'a MenuView, answer: &str) -> Option<&'a MenuEntry> {
    if let Ok(index) = answer.parse::<usize>() {
        view.entries.get(index)
    } else {
        view.entries
            .iter()
            .find(|entry| entry.display_label() == answer)
    }
}

fn finish(ctx: &mut WmCtx<'_>, generation: u64, presentation: TrayMenuPresentation, answer: &str) {
    let wm = ctx.core_mut();
    let host = &mut wm.bar.systray_host.instantmenu;
    if generation != host.presenter.generation {
        return;
    }
    host.presenter.source = None;
    if let Some(child) = host.presenter.child.take() {
        crate::util::reap_child_async(child);
    }
    // A D-Bus update can arrive before reconciliation cancels the old source.
    // Validate the model revision as well as the process generation.
    if presentation.revision != wm.bar.systray_host.menu.revision()
        || !host.hosting(wm.state.config.systray.menu_backend)
    {
        return;
    }
    let Some(entry) = selected_entry(&presentation.view, answer) else {
        close_session(wm);
        return;
    };
    if entry.separator || !entry.enabled {
        host.presented = None;
        wm.bar.systray_host.menu.invalidate_presentation();
        return;
    }
    let Some(runtime) = wm.bar.systray_host.runtime.as_ref() else {
        close_session(wm);
        return;
    };
    runtime.dispatch_menu_action(presentation.session_id, entry.action, &presentation.view);
    // Keep the presentation stamp until the worker acknowledges the action.
    // This prevents respawning the previous level while a submenu is loading.
}

fn close_session(wm: &mut WmCore) {
    crate::systray::close_menu(wm);
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
    use crate::systray::MenuToggle;

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
        let mut host = InstantMenuHost::new();
        assert!(!host.hosting(TrayMenuBackend::Auto), "not scanned yet");

        host.available = Some(true);
        assert!(host.hosting(TrayMenuBackend::Auto));
        assert!(host.hosting(TrayMenuBackend::InstantMenu));
        assert!(!host.hosting(TrayMenuBackend::StatusBar));

        host.refresh_availability(TrayMenuBackend::StatusBar);
        assert_eq!(host.available, None, "a backend change rescans");
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

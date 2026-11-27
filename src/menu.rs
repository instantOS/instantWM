//! Central registry for WM-owned instantMENU sessions.
//!
//! Each chooser used to own a presenter field that both backends had to
//! install separately. [`MenuManager`] owns the event source once; a chooser
//! starts a session with an [`ActiveMenu`] describing how an answer is applied.
//!
//! Shared rows/options and shortcut hints live here so every chooser formats
//! instantMENU metadata the same way.

use std::ffi::OsStr;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::actions::{KeyAction, NamedAction};
use crate::contexts::WmCtx;
use crate::menu_presenter::{self, MenuHost};
use crate::types::{Key, Modifier, Monitor, WindowId};

pub(crate) const INSTANTMENU_BIN: &str = "instantmenu";
pub(crate) const MAX_MENU_LABEL_CHARS: usize = 200;

/// The chooser an open menu session belongs to.
pub(crate) enum ActiveMenu {
    WindowSwitcher(Vec<WindowId>),
    Layout(crate::bar::layout_menu::LayoutMenuState),
    WindowProperties(crate::window_properties::WindowPropertiesState),
}

impl ActiveMenu {
    /// Apply the presenter's answer. The snapshot captured at open time is the
    /// only source of valid selections.
    pub(crate) fn apply(self, ctx: &mut WmCtx<'_>, answer: &str) -> Result<(), String> {
        match self {
            Self::WindowSwitcher(windows) => crate::window_menu::apply(ctx, windows, answer),
            Self::Layout(state) => crate::bar::layout_menu::apply(ctx, state, answer),
            Self::WindowProperties(state) => crate::window_properties::apply(ctx, state, answer),
        }
    }
}

/// Owns the event source and child for every WM-owned menu.
#[derive(Default)]
pub(crate) struct MenuManager {
    pub(crate) host: MenuHost<ActiveMenu>,
}

impl MenuManager {
    pub(crate) fn install<T: 'static>(
        &mut self,
        handle: calloop::LoopHandle<'static, T>,
        context: for<'a> fn(&'a mut T) -> WmCtx<'a>,
    ) {
        menu_presenter::install(handle, &mut self.host, context, finish);
    }

    pub(crate) fn is_open(&self) -> bool {
        self.host.is_open()
    }

    /// Start a session unless one is already showing. Repeated triggers must
    /// not spawn competing menus or discard the first session's result.
    pub(crate) fn start(&mut self, spec: MenuSpec, session: ActiveMenu) -> Result<(), String> {
        if self.is_open() {
            return Ok(());
        }
        menu_presenter::start(&mut self.host, spec.command(), session, spec.input())
    }
}

/// Event-source callback for every chooser.
pub(crate) fn finish(ctx: &mut WmCtx<'_>, generation: u64, session: ActiveMenu, answer: &str) {
    if !ctx
        .core_mut()
        .menus
        .host
        .complete(generation, menu_presenter::finish_child)
    {
        return;
    }
    if let Err(error) = session.apply(ctx, answer) {
        log::warn!("menu selection: {error}");
    }
}

/// One presenter row.
pub(crate) struct MenuChoice {
    value: Option<String>,
    label: String,
    icon: Option<&'static str>,
    color: Option<MenuColor>,
    shortcut: Option<String>,
}

impl MenuChoice {
    pub(crate) fn item(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: Some(sanitize_value(&value.into())),
            label: label.into(),
            icon: None,
            color: None,
            shortcut: None,
        }
    }

    pub(crate) fn heading(label: impl Into<String>) -> Self {
        Self {
            value: None,
            label: label.into(),
            icon: None,
            color: None,
            shortcut: None,
        }
    }

    pub(crate) fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub(crate) fn color(mut self, color: MenuColor) -> Self {
        self.color = Some(color);
        self
    }

    pub(crate) fn shortcut(mut self, shortcut: Option<String>) -> Self {
        self.shortcut = shortcut;
        self
    }

    pub(crate) fn to_line(&self) -> String {
        let mut line = String::from("{");
        let mut token = |key: &str, value: &str| {
            if line.len() > 1 {
                line.push(' ');
            }
            line.push_str(key);
            line.push_str(value);
        };
        if self.value.is_none() {
            token("", "heading");
        }
        if let Some(color) = self.color {
            token("", color.as_str());
        }
        if let Some(icon) = self.icon {
            token("icon=", icon);
        }
        if let Some(value) = &self.value {
            token("value=", value);
            if let Some(shortcut) = &self.shortcut {
                token("shortcut=", shortcut);
            }
        }
        line.push_str("} ");
        line.push_str(&sanitize_label(&self.label, MAX_MENU_LABEL_CHARS));
        line
    }
}

/// Cell color understood by instantMENU.
///
/// The full palette stays available for future choosers even when the current
/// menus only use a subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum MenuColor {
    Normal,
    Fade,
    Blue,
    Green,
    Yellow,
    Red,
}

impl MenuColor {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Fade => "fade",
            Self::Blue => "blue",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Red => "red",
        }
    }
}

/// Anchor understood by instantMENU.
///
/// The full set stays available for future choosers even when the current
/// menus only anchor at the center or below the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum MenuPosition {
    Center,
    TopLeft,
    Top,
    TopRight,
    Left,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl MenuPosition {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::TopLeft => "top-left",
            Self::Top => "top",
            Self::TopRight => "top-right",
            Self::Left => "left",
            Self::Right => "right",
            Self::BottomLeft => "bottom-left",
            Self::Bottom => "bottom",
            Self::BottomRight => "bottom-right",
        }
    }
}

/// Command-line options for instantMENU.
pub(crate) struct MenuOptions {
    pub(crate) prompt: Option<String>,
    pub(crate) placeholder: Option<String>,
    pub(crate) position: MenuPosition,
    pub(crate) border_width: u32,
    pub(crate) insensitive: bool,
    pub(crate) reject_no_match: bool,
    pub(crate) monitor: Option<String>,
    pub(crate) x_offset: Option<i32>,
    pub(crate) y_offset: Option<i32>,
    /// `Some("auto")` or a pixel width such as `Some("700")`.
    pub(crate) width: Option<String>,
    /// `Some("auto")` or a pixel height.
    pub(crate) line_height: Option<String>,
    pub(crate) columns: Option<u32>,
    pub(crate) lines: Option<u32>,
    pub(crate) alt_tab: bool,
    pub(crate) preselect: Option<String>,
}

impl Default for MenuOptions {
    fn default() -> Self {
        Self {
            prompt: None,
            placeholder: None,
            position: MenuPosition::Center,
            border_width: 4,
            insensitive: true,
            reject_no_match: true,
            monitor: None,
            x_offset: None,
            y_offset: None,
            width: None,
            line_height: None,
            columns: None,
            lines: None,
            alt_tab: false,
            preselect: None,
        }
    }
}

impl MenuOptions {
    pub(crate) fn anchor_below_bar(&mut self, monitor: &Monitor, local_x: i32) {
        self.position = MenuPosition::TopLeft;
        self.monitor = Some(monitor.num.max(0).to_string());
        self.x_offset = Some(monitor.work_rect().x - monitor.monitor_rect.x + local_x);
        self.y_offset = Some(monitor.bar_y() + monitor.bar_height - monitor.monitor_rect.y);
    }

    pub(crate) fn command(&self, presenter: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(presenter);
        command.args([
            "--border-width",
            &self.border_width.to_string(),
            "--position",
            self.position.as_str(),
        ]);
        if self.insensitive {
            command.arg("--insensitive");
        }
        if self.reject_no_match {
            command.arg("--reject-no-match");
        }
        if let Some(prompt) = &self.prompt {
            command.args(["--prompt", prompt]);
        }
        if let Some(placeholder) = &self.placeholder {
            command.args(["--placeholder", placeholder]);
        }
        if let Some(monitor) = &self.monitor {
            command.args(["--monitor", monitor]);
        }
        if let Some(offset) = self.x_offset {
            command.args(["--x-offset", &offset.to_string()]);
        }
        if let Some(offset) = self.y_offset {
            command.args(["--y-offset", &offset.to_string()]);
        }
        if let Some(width) = &self.width {
            command.args(["--width", width]);
        }
        if let Some(line_height) = &self.line_height {
            command.args(["--line-height", line_height]);
        }
        if let Some(columns) = self.columns {
            command.args(["--columns", &columns.to_string()]);
        }
        if let Some(lines) = self.lines {
            command.args(["--lines", &lines.to_string()]);
        }
        if self.alt_tab {
            command.arg("--alt-tab");
        }
        if let Some(preselect) = &self.preselect {
            command.args(["--preselect", preselect]);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .process_group(0);
        command
    }
}

/// A complete instantMENU invocation for a WM chooser.
pub(crate) struct MenuSpec {
    pub(crate) options: MenuOptions,
    pub(crate) choices: Vec<MenuChoice>,
}

impl MenuSpec {
    pub(crate) fn command(&self) -> Command {
        self.options.command(INSTANTMENU_BIN)
    }

    pub(crate) fn input(&self) -> String {
        menu_input(&self.choices)
    }
}

pub(crate) fn menu_input(choices: &[MenuChoice]) -> String {
    let mut input = String::new();
    for choice in choices {
        input.push_str(&choice.to_line());
        input.push('\n');
    }
    input
}

/// Make untrusted text safe for one presenter row: no control characters, no
/// brace syntax, and a bounded length.
pub(crate) fn sanitize_label(text: &str, max_chars: usize) -> String {
    text.chars()
        .take(max_chars)
        .filter(|character| !matches!(character, '{' | '}'))
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// Values are short identifiers, never presentation text. Anything that could
/// terminate a row or split the metadata field becomes a dash.
fn sanitize_value(value: &str) -> String {
    let mut value: String = value
        .chars()
        .take(64)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':') {
                character
            } else {
                '-'
            }
        })
        .collect();
    if value.is_empty() {
        value.push_str("item");
    }
    value
}

/// First binding for `action` that instantMENU can render as a chord.
pub(crate) fn shortcut_hint(bindings: &[Key], action: &NamedAction) -> Option<String> {
    shortcut_hint_where(bindings, |bound| bound == action)
}

/// Like [`shortcut_hint`] but lets the caller define action equivalence, so a
/// chooser can accept alternative bindings that perform the same command.
pub(crate) fn shortcut_hint_where(
    bindings: &[Key],
    predicate: impl Fn(&NamedAction) -> bool,
) -> Option<String> {
    bindings
        .iter()
        .filter(|binding| match &binding.action {
            KeyAction::Named(bound) => predicate(bound),
            _ => false,
        })
        .find_map(binding_shortcut)
}

/// Translate only chords supported by instantMENU; invalid metadata would
/// otherwise cause it to treat the whole decorated row as ordinary text.
fn binding_shortcut(binding: &Key) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::KeyAction;
    use crate::layouts::LayoutCommand;
    use crate::types::{KeybindOrigin, Keysym};
    use std::os::unix::process::CommandExt;

    fn binding(modifiers: &str, key: &str, action: NamedAction) -> Key {
        Key {
            mod_mask: modifiers.parse().unwrap(),
            keysym: Keysym::from_name(key).unwrap(),
            action: KeyAction::Named(action),
            origin: KeybindOrigin::User,
        }
    }

    #[test]
    fn markup_serializes_headings_items_and_metadata() {
        assert_eq!(
            MenuChoice::heading("Windows").to_line(),
            "{heading} Windows"
        );
        assert_eq!(
            MenuChoice::heading("Windows")
                .color(MenuColor::Blue)
                .icon("mdwindow")
                .to_line(),
            "{heading blue icon=mdwindow} Windows"
        );
        assert_eq!(
            MenuChoice::item("2", "Editor")
                .icon("codwindow")
                .color(MenuColor::Green)
                .shortcut(Some("alt-tab".into()))
                .to_line(),
            "{green icon=codwindow value=2 shortcut=alt-tab} Editor"
        );
        assert_eq!(MenuChoice::item("x", "Plain").to_line(), "{value=x} Plain");
        assert_eq!(
            MenuChoice::item("x", "Faded")
                .color(MenuColor::Fade)
                .to_line(),
            "{fade value=x} Faded"
        );

        let heading = MenuChoice::heading("No value")
            .shortcut(Some("ignored".into()))
            .to_line();
        assert!(!heading.contains("value="));
        assert!(!heading.contains("shortcut="));
    }

    #[test]
    fn labels_lose_braces_control_characters_and_excess_length() {
        assert_eq!(sanitize_label("a\nb\tc", 200), "a b c");
        assert_eq!(
            sanitize_label("{injected} value=evil}", 200),
            "injected value=evil"
        );
        assert_eq!(sanitize_label(&"x".repeat(300), 200).chars().count(), 200);

        let line = MenuChoice::item("1", "{bad}\nvalue=2").to_line();
        assert_eq!(line, "{value=1} bad value=2");
        assert_eq!(line.matches('}').count(), 1);
    }

    #[test]
    fn unsafe_values_become_menu_identifiers() {
        assert_eq!(MenuChoice::item("2 3", "x").to_line(), "{value=2-3} x");
        assert_eq!(MenuChoice::item("", "x").to_line(), "{value=item} x");
        let line = MenuChoice::item("{a}\n", "x").to_line();
        let metadata = line.split_once('}').unwrap().0;
        assert_eq!(metadata, "{value=-a--");
        assert!(!metadata.chars().any(char::is_whitespace));
    }

    #[test]
    fn command_builds_common_instantmenu_options() {
        let spec = MenuSpec {
            options: MenuOptions {
                prompt: Some("windows".into()),
                placeholder: Some("Choose a window".into()),
                position: MenuPosition::TopLeft,
                monitor: Some("1".into()),
                x_offset: Some(0),
                y_offset: Some(-4),
                width: Some("auto".into()),
                line_height: Some("auto".into()),
                columns: Some(2),
                lines: Some(4),
                alt_tab: true,
                preselect: Some("2".into()),
                ..Default::default()
            },
            choices: vec![MenuChoice::item("2", "Editor")],
        };
        let command = spec.command();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        for pair in [
            ["--prompt", "windows"],
            ["--placeholder", "Choose a window"],
            ["--position", "top-left"],
            ["--monitor", "1"],
            ["--x-offset", "0"],
            ["--y-offset", "-4"],
            ["--width", "auto"],
            ["--line-height", "auto"],
            ["--columns", "2"],
            ["--lines", "4"],
            ["--preselect", "2"],
            ["--border-width", "4"],
        ] {
            assert!(args.windows(2).any(|args| args == pair), "missing {pair:?}");
        }
        assert!(args.contains(&"--insensitive"));
        assert!(args.contains(&"--reject-no-match"));
        assert!(args.contains(&"--alt-tab"));
        assert_eq!(spec.input(), "{value=2} Editor\n");
    }

    #[test]
    fn default_options_keep_the_existing_center_anchor() {
        let spec = MenuSpec {
            options: MenuOptions::default(),
            choices: Vec::new(),
        };
        let args: Vec<_> = spec
            .command()
            .get_args()
            .map(|arg| arg.to_str().unwrap().to_owned())
            .collect();
        assert!(args.windows(2).any(|pair| pair == ["--position", "center"]));
        assert!(args.contains(&"--insensitive".to_string()));
        assert!(args.contains(&"--reject-no-match".to_string()));
        assert!(!args.contains(&"--alt-tab".to_string()));
        assert!(!args.contains(&"--x-offset".to_string()));
        assert!(!args.contains(&"--y-offset".to_string()));
        assert_eq!(spec.input(), "");
    }

    #[test]
    fn manager_starts_only_one_session() {
        let mut menus = MenuManager::default();
        assert!(!menus.is_open());
        assert!(
            menus
                .start(
                    MenuSpec {
                        options: MenuOptions::default(),
                        choices: Vec::new(),
                    },
                    ActiveMenu::WindowSwitcher(Vec::new()),
                )
                .is_err(),
            "an uninstalled manager must report the missing event source"
        );

        let child = Command::new("sh")
            .args(["-c", "exec sleep 30"])
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id();
        menus.host.child = Some(child);
        assert!(menus.is_open());
        menus
            .start(
                MenuSpec {
                    options: MenuOptions::default(),
                    choices: Vec::new(),
                },
                ActiveMenu::WindowSwitcher(Vec::new()),
            )
            .unwrap();
        assert_eq!(menus.host.child.as_ref().map(|child| child.id()), Some(pid));
        menus.host.cancel();
        assert!(!menus.is_open());
    }

    #[test]
    fn shortcut_hints_follow_actions_and_chord_support() {
        let bindings = [
            binding("Super", "c", NamedAction::WindowMenu),
            binding(
                "Control",
                "F3",
                NamedAction::ResetLayoutTo(LayoutCommand::Tile),
            ),
        ];
        assert_eq!(
            shortcut_hint(&bindings, &NamedAction::WindowMenu),
            Some("super-c".into())
        );
        assert_eq!(
            shortcut_hint_where(&bindings, |action| matches!(
                action,
                NamedAction::ResetLayoutTo(_)
            )),
            Some("ctrl-f3".into())
        );
        assert_eq!(
            shortcut_hint(&bindings, &NamedAction::WindowMenuReverse),
            None
        );

        let unsupported = [
            binding("Mod3", "g", NamedAction::WindowMenu),
            binding("", "Return", NamedAction::WindowMenu),
            binding("Control", "c", NamedAction::WindowMenu),
        ];
        assert_eq!(shortcut_hint(&unsupported, &NamedAction::WindowMenu), None);
        assert_eq!(
            shortcut_hint(
                &[binding("Alt", "Prior", NamedAction::WindowMenu)],
                &NamedAction::WindowMenu
            ),
            Some("alt-page-up".into())
        );
    }
}

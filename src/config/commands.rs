//! External command definitions for keybindings and button handlers.
//!
//! # Design
//!
//! All launchable commands are collected in [`ExternalCommands`].  Each field
//! is a `&'static [&'static str]` slice — the first element is the executable,
//! the rest are its arguments, exactly as you would pass them to `execvp`.
//!
//! Keybindings and button handlers reference commands through the typed
//! [`Cmd`] enum, which is resolved to a concrete slice at runtime via
//! [`ExternalCommands::get`].  This replaces the old `CMD_*: usize` integer
//! constants, which were fragile (renumbering broke everything silently) and
//! gave no information at a glance about what they pointed to.
//!
//! # Adding a new command
//!
//! 1. Add a `pub` field to [`ExternalCommands`].
//! 2. Add a variant to [`Cmd`] with a matching name.
//! 3. Add the `Cmd::YourVariant => &self.your_field` arm in [`ExternalCommands::get`].
//! 4. Use `Cmd::YourVariant` in your keybinding / button definition.

// ---------------------------------------------------------------------------
// Scratchpad
// ---------------------------------------------------------------------------

/// Window class used for the default scratchpad terminal.
/// Referenced by [`ExternalCommands::term_scratch`] and by the window rules.
pub const SCRATCHPAD_CLASS: &str = "scratchpad_default";

// ---------------------------------------------------------------------------
// Typed command selector
// ---------------------------------------------------------------------------

/// Identifies a specific command entry inside [`ExternalCommands`].
///
/// Use this to reference a command in keybindings and button handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    /// Placeholder / sentinel — resolves to an empty slice.
    Default,
    Term,
    TermScratch,
    InstantMenu,
    ClipMenu,
    Smart,
    InstantMenuSt,
    QuickMenu,
    InstantAssist,
    InstantRepeat,
    InstantPacman,
    InstantShare,
    Nautilus,
    Slock,
    OneKeyLock,
    LangSwitch,
    OsLock,
    Help,
    Search,
    KeyLayoutSwitch,
    ISwitch,
    InstantSwitch,
    CaretInstantSwitch,
    InstantSkippy,
    Onboard,
    InstantShutdown,
    SystemMonitor,
    Notify,
    Yazi,
    Panther,
    ControlCenter,
    Display,
    PavuControl,
    InstantSettings,
    Code,
    StartMenu,
    Scrot,
    FScrot,
    ClipScrot,
    FClipScrot,
    Firefox,
    Editor,
    PlayerNext,
    PlayerPrevious,
    PlayerPause,
    Spoticli,
    UpVol,
    DownVol,
    MuteVol,
    UpBright,
    DownBright,
    /// Passed to `name_tag` / `scratchpad-*` IPC commands as a generic tag arg.
    Tag,
}

// ---------------------------------------------------------------------------
// ExternalCommands
// ---------------------------------------------------------------------------

/// All external commands launched by keybindings and button handlers.
///
/// Each field is a static slice `&[&str]` where `[0]` is the binary and the
/// remaining elements are its arguments.
#[derive(Debug, Clone)]
pub struct ExternalCommands {
    pub instantmenu: &'static [&'static str],
    pub clipmenu: &'static [&'static str],
    pub smart: &'static [&'static str],
    pub instantmenu_st: &'static [&'static str],
    pub term: &'static [&'static str],
    pub term_scratch: &'static [&'static str],
    pub quickmenu: &'static [&'static str],
    pub instantassist: &'static [&'static str],
    pub instantrepeat: &'static [&'static str],
    pub instantpacman: &'static [&'static str],
    pub instantshare: &'static [&'static str],
    pub nautilus: &'static [&'static str],
    pub slock: &'static [&'static str],
    pub onekeylock: &'static [&'static str],
    pub langswitch: &'static [&'static str],
    pub oslock: &'static [&'static str],
    pub help: &'static [&'static str],
    pub search: &'static [&'static str],
    pub keylayoutswitch: &'static [&'static str],
    pub iswitch: &'static [&'static str],
    pub instantswitch: &'static [&'static str],
    pub caretinstantswitch: &'static [&'static str],
    pub instantskippy: &'static [&'static str],
    pub onboard: &'static [&'static str],
    pub instantshutdown: &'static [&'static str],
    pub systemmonitor: &'static [&'static str],
    pub notify: &'static [&'static str],
    pub yazi: &'static [&'static str],
    pub panther: &'static [&'static str],
    pub controlcenter: &'static [&'static str],
    pub display: &'static [&'static str],
    pub pavucontrol: &'static [&'static str],
    pub instantsettings: &'static [&'static str],
    pub code: &'static [&'static str],
    pub startmenu: &'static [&'static str],
    pub scrot: &'static [&'static str],
    pub fscrot: &'static [&'static str],
    pub clipscrot: &'static [&'static str],
    pub fclipscrot: &'static [&'static str],
    pub firefox: &'static [&'static str],
    pub editor: &'static [&'static str],
    pub player_next: &'static [&'static str],
    pub player_previous: &'static [&'static str],
    pub player_pause: &'static [&'static str],
    pub spoticli: &'static [&'static str],
    pub upvol: &'static [&'static str],
    pub downvol: &'static [&'static str],
    pub mutevol: &'static [&'static str],
    pub upbright: &'static [&'static str],
    pub downbright: &'static [&'static str],
}

impl ExternalCommands {
    /// Resolve a [`Cmd`] variant to the corresponding argv slice.
    pub fn get(&self, cmd: Cmd) -> &'static [&'static str] {
        match cmd {
            Cmd::Default => &[],
            Cmd::Tag => &[],
            Cmd::InstantMenu => self.instantmenu,
            Cmd::ClipMenu => self.clipmenu,
            Cmd::Smart => self.smart,
            Cmd::InstantMenuSt => self.instantmenu_st,
            Cmd::Term => self.term,
            Cmd::TermScratch => self.term_scratch,
            Cmd::QuickMenu => self.quickmenu,
            Cmd::InstantAssist => self.instantassist,
            Cmd::InstantRepeat => self.instantrepeat,
            Cmd::InstantPacman => self.instantpacman,
            Cmd::InstantShare => self.instantshare,
            Cmd::Nautilus => self.nautilus,
            Cmd::Slock => self.slock,
            Cmd::OneKeyLock => self.onekeylock,
            Cmd::LangSwitch => self.langswitch,
            Cmd::OsLock => self.oslock,
            Cmd::Help => self.help,
            Cmd::Search => self.search,
            Cmd::KeyLayoutSwitch => self.keylayoutswitch,
            Cmd::ISwitch => self.iswitch,
            Cmd::InstantSwitch => self.instantswitch,
            Cmd::CaretInstantSwitch => self.caretinstantswitch,
            Cmd::InstantSkippy => self.instantskippy,
            Cmd::Onboard => self.onboard,
            Cmd::InstantShutdown => self.instantshutdown,
            Cmd::SystemMonitor => self.systemmonitor,
            Cmd::Notify => self.notify,
            Cmd::Yazi => self.yazi,
            Cmd::Panther => self.panther,
            Cmd::ControlCenter => self.controlcenter,
            Cmd::Display => self.display,
            Cmd::PavuControl => self.pavucontrol,
            Cmd::InstantSettings => self.instantsettings,
            Cmd::Code => self.code,
            Cmd::StartMenu => self.startmenu,
            Cmd::Scrot => self.scrot,
            Cmd::FScrot => self.fscrot,
            Cmd::ClipScrot => self.clipscrot,
            Cmd::FClipScrot => self.fclipscrot,
            Cmd::Firefox => self.firefox,
            Cmd::Editor => self.editor,
            Cmd::PlayerNext => self.player_next,
            Cmd::PlayerPrevious => self.player_previous,
            Cmd::PlayerPause => self.player_pause,
            Cmd::Spoticli => self.spoticli,
            Cmd::UpVol => self.upvol,
            Cmd::DownVol => self.downvol,
            Cmd::MuteVol => self.mutevol,
            Cmd::UpBright => self.upbright,
            Cmd::DownBright => self.downbright,
        }
    }
}

// ---------------------------------------------------------------------------
// Default command definitions
// ---------------------------------------------------------------------------

/// Build the default [`ExternalCommands`] table.
///
/// Most entries delegate to instantOS helper scripts or per-user config files
/// under `~/.config/instantos/default/`.
pub fn default_commands() -> ExternalCommands {
    ExternalCommands {
        instantmenu: &["instantmenu_run"],
        clipmenu: &["instantclipmenu"],
        smart: &["instantmenu_smartrun"],
        instantmenu_st: &["instantmenu_run_st"],
        term: &["kitty"],
        term_scratch: &[".config/instantos/default/terminal", "-c", SCRATCHPAD_CLASS],
        quickmenu: &["quickmenu"],
        instantassist: &["ins", "assist"],
        instantrepeat: &["instantrepeat"],
        instantpacman: &["instantpacman"],
        instantshare: &["instantshare", "snap"],
        nautilus: &[".config/instantos/default/filemanager"],
        slock: &[".config/instantos/default/lockscreen"],
        onekeylock: &["ilock", "-o"],
        langswitch: &["ilayout"],
        oslock: &["instantlock", "-o"],
        help: &["instanthotkeys", "gui"],
        search: &["instantsearch"],
        keylayoutswitch: &["instantkeyswitch"],
        iswitch: &["iswitch"],
        instantswitch: &[
            "rofi",
            "-show",
            "window",
            "-kb-row-down",
            "Alt+Tab,Down",
            "-kb-row-up",
            "Alt+Ctrl+Tab,Up",
            "-kb-accept-entry",
            "!Alt_L,!Alt+Tab,Return",
            "-me-select-entry",
            "",
            "-me-accept-entry",
            "MousePrimary",
        ],
        caretinstantswitch: &[
            "rofi",
            "-show",
            "window",
            "-kb-row-down",
            "Alt+Tab,Down",
            "-kb-row-up",
            "Alt+Ctrl+Tab,Up",
            "-kb-accept-entry",
            "!Alt_L,!Alt+Tab,Return",
            "-me-select-entry",
            "",
            "-me-accept-entry",
            "MousePrimary",
            "-theme",
            "/usr/share/instantdotfiles/rootconfig/rofi/appmenu.rasi",
        ],
        instantskippy: &["instantskippy"],
        onboard: &["onboard"],
        instantshutdown: &["instantshutdown"],
        systemmonitor: &[".config/instantos/default/systemmonitor"],
        notify: &["instantnotify"],
        yazi: &[".config/instantos/default/termfilemanager"],
        panther: &[".config/instantos/default/appmenu"],
        controlcenter: &["ins", "settings", "--gui"],
        display: &["instantdisper"],
        pavucontrol: &["pavucontrol"],
        instantsettings: &["ins", "settings", "--gui"],
        code: &["instantutils", "open", "graphicaleditor"],
        startmenu: &["instantstartmenu"],
        scrot: &["ins", "assist", "run", "sp"],
        fscrot: &["ins", "assist", "run", "sm"],
        clipscrot: &["ins", "assist", "run", "sc"],
        fclipscrot: &["ins", "assist", "run", "sf"],
        firefox: &[".config/instantos/default/browser"],
        editor: &[".config/instantos/default/editor"],
        player_next: &["playerctl", "next"],
        player_previous: &["playerctl", "previous"],
        player_pause: &["playerctl", "play-pause"],
        spoticli: &["spoticli", "m"],
        upvol: &["ins", "assist", "volume", "+"],
        downvol: &["ins", "assist", "volume", "-"],
        mutevol: &["ins", "assist", "volume", "mute"],
        upbright: &["ins", "assist", "bright", "+"],
        downbright: &["ins", "assist", "bright", "-"],
    }
}

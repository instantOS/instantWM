//! instantMENU is also the WM's layout/menu presenter. All subprocess and disk
//! operations run off the compositor thread. Cancellation kills the live menu.
use super::{Shared, Shortcut, Trigger};
use crate::menu::{MAX_MENU_LABEL_CHARS, MenuChoice, MenuColor, MenuOptions, menu_input};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

pub(super) struct Dialog {
    presenter: std::path::PathBuf,
    cancelled: AtomicBool,
    failed: AtomicBool,
    child: Mutex<Option<Child>>,
}

/// Roomy rows (2.5x font height) match the layout chooser while the list still
/// fits comfortably. Past this, keep instantMENU's compact default so a long
/// shortcut list leaves the menu on screen.
const ROOMY_LINE_LIMIT: usize = 10;
const MAX_VISIBLE_LINES: usize = 14;
const MAX_DIALOG_TEXT_CHARS: usize = 180;

fn menu_command(
    presenter: &std::path::Path,
    prompt: &str,
    entries: &str,
    input: Option<&str>,
) -> Command {
    let lines = entries.lines().count().clamp(1, MAX_VISIBLE_LINES);
    let mut command = MenuOptions {
        width: Some("auto".into()),
        lines: Some(lines as u32),
        placeholder: Some(clean(prompt, MAX_DIALOG_TEXT_CHARS)),
        line_height: (lines <= ROOMY_LINE_LIMIT).then(|| "auto".into()),
        reject_no_match: input.is_none(),
        ..Default::default()
    }
    .command(presenter);
    command.stderr(Stdio::null());
    if let Some(input) = input {
        command.args(["--input-only", "--initial-text", input]);
    }
    command
}

impl Dialog {
    pub fn new(presenter: std::path::PathBuf) -> Self {
        Self {
            presenter,
            cancelled: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            child: Mutex::new(None),
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn menu(&self, prompt: &str, entries: &str, input: Option<&str>) -> Option<String> {
        if self.cancelled() {
            return None;
        }
        let mut command = menu_command(&self.presenter, prompt, entries, input);
        let stdin = {
            let mut slot = self.child.lock().unwrap();
            // Close may race process creation; serialize both against this slot.
            if self.cancelled() {
                return None;
            }
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    self.failed.store(true, Ordering::SeqCst);
                    log::error!("global shortcuts: cannot open instantmenu: {error}");
                    return None;
                }
            };
            let stdin = child.stdin.take();
            *slot = Some(child);
            stdin
        };
        // Cancellation must be able to kill the child even if its stdin is
        // full or it exits before reading all menu entries.
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(entries.as_bytes());
        }
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            let mut slot = self.child.lock().unwrap();
            let child = slot.as_mut()?;
            if self.cancelled() || Instant::now() >= deadline {
                if !self.cancelled() {
                    self.failed.store(true, Ordering::SeqCst);
                }
                let _ = child.kill();
                let _ = child.wait();
                *slot = None;
                return None;
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut child = slot.take().unwrap();
                    let _ = child.wait();
                    if !status.success() {
                        return None;
                    }
                    let mut answer = String::new();
                    child
                        .stdout
                        .take()?
                        .take(4096)
                        .read_to_string(&mut answer)
                        .ok()?;
                    return Some(answer.trim().to_owned());
                }
                Err(_) => {
                    self.failed.store(true, Ordering::SeqCst);
                    let _ = child.kill();
                    let _ = child.wait();
                    *slot = None;
                    return None;
                }
                Ok(None) => {}
            }
            drop(slot);
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Dialog {
    fn drop(&mut self) {
        if let Some(child) = self.child.get_mut().unwrap().as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn clean(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() && *ch != '{' && *ch != '}')
        .take(limit)
        .collect()
}

/// Context rows use `heading`, which instantMENU skips during selection.
/// Keep chords in labels: `shortcut=` would register menu accelerators.
fn approval_entries(app: &str, shortcuts: &[Shortcut]) -> String {
    let mut choices = vec![
        MenuChoice::heading(clean(app, MAX_DIALOG_TEXT_CHARS))
            .color(MenuColor::Blue)
            .icon("mdkeyboard"),
        MenuChoice::heading("These shortcuts work while other applications are focused")
            .color(MenuColor::Fade),
        MenuChoice::heading("Permission"),
        MenuChoice::item("allow", "Allow these global shortcuts")
            .color(MenuColor::Green)
            .icon("mdcheck"),
        MenuChoice::item("cancel", "Cancel").icon("mdclose"),
        MenuChoice::heading("Bindings: select to edit").color(MenuColor::Blue),
    ];
    if shortcuts.is_empty() {
        choices.push(MenuChoice::heading("No shortcuts requested").color(MenuColor::Fade));
    }
    for (index, shortcut) in shortcuts.iter().enumerate() {
        let trigger = shortcut
            .trigger
            .map_or_else(|| "Disabled".into(), |trigger| trigger.text());
        let description_limit = MAX_MENU_LABEL_CHARS.saturating_sub(trigger.chars().count() + 2);
        let description = clean(
            &shortcut.description,
            description_limit.min(MAX_DIALOG_TEXT_CHARS),
        );
        let mut choice =
            MenuChoice::item(format!("edit{index}"), format!("{description}: {trigger}"));
        if shortcut.trigger.is_some() {
            choice = choice.icon("mdkeyboard");
        } else {
            choice = choice.color(MenuColor::Fade).icon("mdkeyboardoff");
        }
        choices.push(choice);
    }
    menu_input(&choices)
}

/// Return a complete user-approved set. Nothing becomes active until the final
/// confirmation; closing any menu leaves the previous bindings untouched.
pub(super) fn configure(
    shared: &Shared,
    path: &str,
    app: &str,
    mut shortcuts: Vec<Shortcut>,
    dialog: &Dialog,
) -> Option<Vec<Shortcut>> {
    let _serial = loop {
        if dialog.cancelled() {
            return None;
        }
        match shared.ui.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(std::sync::TryLockError::Poisoned(_)) => return None,
        }
    };
    let app = if app.is_empty() {
        "Unidentified application"
    } else {
        app
    };
    loop {
        let entries = approval_entries(app, &shortcuts);
        let answer = dialog.menu("Global shortcuts: select a binding to edit", &entries, None)?;
        if answer == "cancel" {
            return None;
        }
        if answer == "allow" {
            let registry = shared.registry.lock().unwrap();
            let mut used = std::collections::HashSet::new();
            if shortcuts.iter().any(|shortcut| {
                shortcut.trigger.is_some_and(|trigger| {
                    Shared::conflict(&registry, path, &shortcut.id, trigger)
                        || !used.insert(trigger)
                })
            }) {
                drop(registry);
                dialog.menu(
                    "A shortcut is already in use. Edit it before allowing.",
                    "{heading yellow icon=mdinformation} Resolve conflicting bindings before allowing\n{icon=mdarrowleft value=ok} Back to shortcuts\n",
                    None,
                )?;
                continue;
            }
            return Some(shortcuts);
        }
        let Some(index) = answer
            .strip_prefix("edit")
            .and_then(|text| text.parse::<usize>().ok())
            .filter(|index| *index < shortcuts.len())
        else {
            continue;
        };
        let shortcut = &mut shortcuts[index];
        let mut input = shortcut
            .trigger
            .map(Trigger::text)
            .or_else(|| shortcut.preferred.clone())
            .unwrap_or_default();
        loop {
            let text = dialog.menu(
                &format!(
                    "{}: type e.g. CTRL+ALT+space; empty disables",
                    clean(&shortcut.description, MAX_DIALOG_TEXT_CHARS)
                ),
                "",
                Some(&input),
            )?;
            if text.is_empty() {
                shortcut.trigger = None;
                break;
            }
            let parsed = Trigger::parse(&text).and_then(|trigger| {
                if Shared::conflict(
                    &shared.registry.lock().unwrap(),
                    path,
                    &shortcut.id,
                    trigger,
                ) {
                    Err("Shortcut already in use".into())
                } else {
                    Ok(trigger)
                }
            });
            match parsed {
                Ok(trigger) => {
                    shortcut.trigger = Some(trigger);
                    break;
                }
                Err(error) => {
                    dialog.menu(
                        &error,
                        "{heading yellow icon=mdinformation} Binding could not be saved\n{icon=mdrefresh value=ok} Try again\n",
                        None,
                    )?;
                    input = text;
                }
            }
        }
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use crate::shortcuts::{Session, Shortcuts};
    use std::sync::Arc;

    #[test]
    fn cancelling_a_busy_dialog_releases_the_registry_before_waiting() {
        let shortcuts = Shortcuts::default();
        let dialog = Arc::new(Dialog::new("unused".into()));
        shortcuts.shared.registry.lock().unwrap().sessions.insert(
            "session".into(),
            Session {
                app: String::new(),
                owner: String::new(),
                shortcuts: Vec::new(),
                bound: false,
                generation: 0,
                dialog: Some(dialog.clone()),
            },
        );
        // Model a child spawn/wait holding the process slot while another
        // thread closes its portal session.
        let child = dialog.child.lock().unwrap();
        let shared = shortcuts.shared.clone();
        let task = std::thread::spawn(move || shared.remove_session("session"));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !dialog.cancelled() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let cancelled = dialog.cancelled();
        let registry_available = shortcuts.shared.registry.try_lock().is_ok();
        drop(child);
        task.join().unwrap();
        assert!(cancelled, "session closure did not start cancellation");
        assert!(
            registry_available,
            "cancellation held the input registry while waiting for its child"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_context_is_inert_but_disabled_bindings_remain_editable() {
        let entries = approval_entries(
            "test.app\n{value=allow}",
            &[
                Shortcut {
                    id: "one".into(),
                    description: "Talk\n{heading}".into(),
                    preferred: None,
                    trigger: Some(Trigger::parse("CTRL+space").unwrap()),
                },
                Shortcut {
                    id: "two".into(),
                    description: "Muted action".into(),
                    preferred: None,
                    trigger: None,
                },
            ],
        );
        assert_eq!(entries.lines().count(), 8);
        let headings: Vec<_> = entries
            .lines()
            .filter(|line| line.starts_with("{heading"))
            .collect();
        assert_eq!(headings.len(), 4);
        for heading in headings {
            let metadata = heading.split_once('}').unwrap().0;
            assert!(!metadata.contains("value="));
        }
        assert!(entries.contains("{icon=mdkeyboard value=edit0} Talkheading: CTRL+space\n"));
        assert!(entries.contains("{fade icon=mdkeyboardoff value=edit1} Muted action: Disabled\n"));
        assert!(!entries.contains("shortcut="));
        assert!(!entries.contains('—'));
        assert_eq!(
            entries.matches("{green icon=mdcheck value=allow}").count(),
            1
        );
    }

    #[test]
    fn empty_requests_have_an_explanatory_heading() {
        assert!(
            approval_entries("test.app", &[]).ends_with("{heading fade} No shortcuts requested\n")
        );
    }

    #[test]
    fn menus_match_case_insensitively_and_have_a_border() {
        let command = menu_command(
            std::path::Path::new("instantmenu"),
            "Global shortcuts: select a binding to edit",
            "{heading blue} Bindings: select to edit\n{icon=mdcheck value=allow} Allow\n",
            None,
        );
        let args: Vec<_> = command
            .get_args()
            .map(|argument| argument.to_str().unwrap())
            .collect();
        assert!(args.windows(2).any(|pair| pair == ["--border-width", "4"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--line-height", "auto"])
        );
        assert!(args.contains(&"--insensitive"));
        assert!(args.contains(&"--reject-no-match"));
        assert!(!args.iter().any(|argument| argument.contains('—')));
    }

    #[test]
    fn shortcut_editor_accepts_input_without_a_matching_row() {
        let command = menu_command(
            std::path::Path::new("instantmenu"),
            "Edit binding",
            "",
            Some("CTRL+space"),
        );
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert!(args.contains(&"--input-only"));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--initial-text", "CTRL+space"])
        );
        assert!(!args.contains(&"--reject-no-match"));
    }

    #[test]
    fn only_short_menus_use_roomy_rows() {
        let entry = "{icon=mdkeyboard value=edit} Binding\n";
        let short = entry.repeat(ROOMY_LINE_LIMIT);
        let long = entry.repeat(ROOMY_LINE_LIMIT + 1);
        for (entries, roomy) in [(&short, true), (&long, false)] {
            let command =
                menu_command(std::path::Path::new("instantmenu"), "prompt", entries, None);
            let args: Vec<_> = command
                .get_args()
                .map(|argument| argument.to_str().unwrap())
                .collect();
            assert_eq!(
                args.windows(2)
                    .any(|pair| pair == ["--line-height", "auto"]),
                roomy
            );
            assert!(args.contains(&"--lines"));
        }
    }
}

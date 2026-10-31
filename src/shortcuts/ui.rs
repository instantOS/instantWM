//! instantMENU is also the WM's layout/menu presenter. All subprocess and disk
//! operations run off the compositor thread. Cancellation kills the live menu.
use super::{Shared, Shortcut, Trigger};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
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
        let mut command = Command::new(&self.presenter);
        command
            .args([
                "--position",
                "center",
                "--width",
                "auto",
                "--lines",
                &entries.lines().count().clamp(1, 14).to_string(),
                "--placeholder",
                &clean(prompt),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        if let Some(input) = input {
            command.args(["--input-only", "--initial-text", input]);
        } else {
            command.arg("--reject-no-match");
        }
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

fn clean(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() && *ch != '{' && *ch != '}')
        .take(180)
        .collect()
}

/// Context rows use `heading`, which instantMENU skips during selection.
/// Keep chords in labels: `shortcut=` would register menu accelerators.
fn approval_entries(app: &str, shortcuts: &[Shortcut]) -> String {
    let mut entries = format!(
        "{{heading blue icon=mdkeyboard}} {}\n\
         {{heading fade}} These shortcuts work while other applications are focused\n\
         {{heading}} Permission\n\
         {{green icon=mdcheck value=allow}} Allow these global shortcuts\n\
         {{icon=mdclose value=cancel}} Cancel\n\
         {{heading blue}} Bindings — select to edit\n",
        clean(app),
    );
    if shortcuts.is_empty() {
        entries.push_str("{heading fade} No shortcuts requested\n");
    }
    for (index, shortcut) in shortcuts.iter().enumerate() {
        let (style, trigger) = shortcut.trigger.map_or_else(
            || ("fade icon=mdkeyboardoff", "Disabled".into()),
            |trigger| ("icon=mdkeyboard", trigger.text()),
        );
        entries.push_str(&format!(
            "{{{style} value=edit{index}}} {}: {trigger}\n",
            clean(&shortcut.description),
        ));
    }
    entries
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
        let answer = dialog.menu(
            "Global shortcuts — select a binding to edit",
            &entries,
            None,
        )?;
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
                    "{} — type e.g. CTRL+ALT+space; empty disables",
                    clean(&shortcut.description)
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
}

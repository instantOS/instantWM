//! Application global shortcuts. Policy and portal sessions are shared by both
//! backends; only native key delivery and X11 passive grabs differ.
mod portal;
mod store;
#[cfg(test)]
mod tests;
mod ui;

use crate::types::{Keysym, ModMask, Modifier};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Trigger {
    pub key: Keysym,
    pub mods: ModMask,
}

impl Trigger {
    /// XDG shortcuts use base-layer XKB names, not GTK accelerator syntax.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut parts: Vec<_> = text.trim().split('+').collect();
        let name = parts.pop().ok_or("Missing key")?;
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("Use an XKB key name, e.g. CTRL+ALT+space".into());
        }
        let mut mods = ModMask::NONE;
        for part in parts {
            let modifier = match part {
                "CTRL" => Modifier::Control,
                "ALT" => Modifier::Alt,
                "SHIFT" => Modifier::Shift,
                "LOGO" => Modifier::Super,
                // Num Lock is deliberately ignored by WM binding matching.
                // Accepting NUM here would silently register a different chord.
                _ => return Err(format!("Unsupported modifier: {part}")),
            };
            if (mods & ModMask::from_modifier(modifier)) != ModMask::NONE {
                return Err("Repeated modifier".into());
            }
            mods |= ModMask::from_modifier(modifier);
        }
        let key = Keysym::from_name(name)
            .map_err(|e| e.to_string())?
            .for_binding();
        if key.is_modifier()
            || matches!(
                key.raw(),
                0xffe5 | 0xff7f | 0xff14 | 0xffe7 | 0xffe8 | 0xffed | 0xffee
            )
        {
            return Err("Choose a non-modifier key".into());
        }
        // Keep emergency recovery and VT switching outside application control.
        if (key == crate::config::keysyms::XK_ESCAPE
            && mods
                == (ModMask::from_modifier(Modifier::Super)
                    | ModMask::from_modifier(Modifier::Shift)))
            || ((mods
                & (ModMask::from_modifier(Modifier::Control)
                    | ModMask::from_modifier(Modifier::Alt)))
                == (ModMask::from_modifier(Modifier::Control)
                    | ModMask::from_modifier(Modifier::Alt))
                && (crate::config::keysyms::XK_F1.raw()..=crate::config::keysyms::XK_F12.raw())
                    .contains(&key.raw()))
            || (0x1008fe01..=0x1008fe0c).contains(&key.raw())
        {
            return Err("Reserved system shortcut".into());
        }
        Ok(Self { key, mods })
    }

    pub fn text(self) -> String {
        let mut parts = Vec::new();
        for (modifier, name) in [
            (Modifier::Super, "LOGO"),
            (Modifier::Control, "CTRL"),
            (Modifier::Shift, "SHIFT"),
            (Modifier::Alt, "ALT"),
        ] {
            if (self.mods & ModMask::from_modifier(modifier)) != ModMask::NONE {
                parts.push(name.to_owned());
            }
        }
        // Display uses printable punctuation; the wire format requires XKB names.
        parts.push(smithay::input::keyboard::xkb::keysym_get_name(
            smithay::input::keyboard::xkb::Keysym::new(self.key.raw()),
        ));
        parts.join("+")
    }
}

#[derive(Clone, Debug)]
struct Shortcut {
    id: String,
    description: String,
    preferred: Option<String>,
    trigger: Option<Trigger>,
}

struct Session {
    app: String,
    owner: String,
    shortcuts: Vec<Shortcut>,
    bound: bool,
    dialog: Option<Arc<ui::Dialog>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Activation {
    session: String,
    id: String,
}

enum Event {
    Key {
        activation: Activation,
        pressed: bool,
        time: u64,
    },
    Changed(String),
    BusGone,
    Stop,
}

#[derive(Default)]
struct Registry {
    sessions: BTreeMap<String, Session>,
    reserved: HashSet<Trigger>,
    active: HashMap<u32, Activation>,
    revision: u64,
}

struct Shared {
    registry: Mutex<Registry>,
    tx: mpsc::Sender<Event>,
    wake: Mutex<Option<calloop::ping::Ping>>,
    epoch: Instant,
    store: Mutex<store::Store>,
    ui: Mutex<()>,
    validation: Mutex<Vec<(u64, tokio::sync::oneshot::Sender<()>)>>,
    presenter: std::path::PathBuf,
}

impl Shared {
    fn changed(&self, registry: &mut Registry) {
        registry.revision = registry.revision.wrapping_add(1);
        if let Some(wake) = self.wake.lock().unwrap().as_ref() {
            wake.ping();
        }
    }

    fn emit(&self, activation: Activation, pressed: bool) {
        let time = self.epoch.elapsed().as_millis() as u64;
        let _ = self.tx.send(Event::Key {
            activation,
            pressed,
            time,
        });
    }

    fn deactivate(&self, registry: &mut Registry) {
        let activations: HashSet<_> = registry
            .active
            .drain()
            .map(|(_, activation)| activation)
            .collect();
        for activation in activations {
            self.emit(activation, false);
        }
    }

    fn deactivate_session(&self, registry: &mut Registry, path: &str) {
        let keys: Vec<_> = registry
            .active
            .iter()
            .filter(|(_, a)| a.session == path)
            .map(|(k, _)| *k)
            .collect();
        let activations: HashSet<_> = keys
            .into_iter()
            .filter_map(|key| registry.active.remove(&key))
            .collect();
        for activation in activations {
            self.emit(activation, false);
        }
    }

    fn remove_session(&self, path: &str) {
        let mut registry = self.registry.lock().unwrap();
        if let Some(session) = registry.sessions.remove(path) {
            if let Some(dialog) = session.dialog {
                dialog.cancel();
            }
            self.deactivate_session(&mut registry, path);
            self.changed(&mut registry);
        }
    }

    fn conflict(registry: &Registry, session: &str, id: &str, trigger: Trigger) -> bool {
        registry.reserved.contains(&trigger)
            || registry.sessions.iter().any(|(path, s)| {
                s.shortcuts.iter().any(|shortcut| {
                    (path != session || shortcut.id != id) && shortcut.trigger == Some(trigger)
                })
            })
    }
}

pub(crate) struct Shortcuts {
    shared: Arc<Shared>,
    rx: Option<mpsc::Receiver<Event>>,
    worker: Option<JoinHandle<()>>,
    seen_revision: u64,
}

impl Default for Shortcuts {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            shared: Arc::new(Shared {
                registry: Mutex::new(Registry::default()),
                tx,
                wake: Mutex::new(None),
                epoch: Instant::now(),
                store: Mutex::new(store::Store::default()),
                ui: Mutex::new(()),
                validation: Mutex::new(Vec::new()),
                presenter: "instantmenu".into(),
            }),
            rx: Some(rx),
            worker: None,
            seen_revision: 0,
        }
    }
}

impl Shortcuts {
    pub fn set_wake(&self, wake: Option<calloop::ping::Ping>) {
        *self.shared.wake.lock().unwrap() = wake;
    }

    pub fn start(&mut self, wake: Option<calloop::ping::Ping>) {
        self.set_wake(wake);
        if self.worker.is_some() {
            return;
        }
        let Some(rx) = self.rx.take() else {
            return;
        };
        let shared = self.shared.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        self.worker = Some(std::thread::spawn(move || {
            if let Err(error) = portal::serve(shared.clone(), rx, None, ready_tx.clone()) {
                let _ = ready_tx.send(Err(error.to_string()));
                log::error!("global shortcuts portal: {error}");
            }
            let paths: Vec<_> = shared
                .registry
                .lock()
                .unwrap()
                .sessions
                .keys()
                .cloned()
                .collect();
            for path in paths {
                shared.remove_session(&path);
            }
        }));
        if let Ok(Err(error)) = ready_rx.recv_timeout(std::time::Duration::from_secs(3)) {
            log::warn!("global shortcuts portal could not start: {error}");
        }
    }

    /// Synchronize conflicts after configuration reloads and withdraw bindings
    /// newly claimed by WM policy. The next portal list reflects the change.
    pub fn sync(&mut self, bindings: &crate::core_state::BindingConfig, numlock: ModMask) -> bool {
        let reserved: HashSet<_> = bindings
            .keys
            .iter()
            .chain(&bindings.desktop_keybinds)
            .chain(bindings.modes.values().flat_map(|mode| &mode.keybinds))
            .map(|key| Trigger {
                key: key.keysym.for_binding(),
                mods: key.mod_mask.cleaned(numlock),
            })
            .collect();
        let mut registry = self.shared.registry.lock().unwrap();
        if reserved != registry.reserved {
            self.shared.deactivate(&mut registry);
            for (path, session) in &mut registry.sessions {
                let mut withdrawn = false;
                for shortcut in &mut session.shortcuts {
                    if shortcut
                        .trigger
                        .is_some_and(|trigger| reserved.contains(&trigger))
                    {
                        shortcut.trigger = None;
                        withdrawn = true;
                    }
                }
                if withdrawn {
                    let _ = self.shared.tx.send(Event::Changed(path.clone()));
                }
            }
            registry.reserved = reserved;
            self.shared.changed(&mut registry);
        }
        let changed = self.seen_revision != registry.revision;
        self.seen_revision = registry.revision;
        changed
    }

    pub fn take_changed(&mut self) -> bool {
        let revision = self.shared.registry.lock().unwrap().revision;
        let changed = self.seen_revision != revision;
        self.seen_revision = revision;
        changed
    }

    /// Complete native registration before replying to a portal bind request.
    pub fn validated(&self) {
        let mut pending = self.shared.validation.lock().unwrap();
        let mut newer = Vec::new();
        for (revision, waiter) in pending.drain(..) {
            if revision <= self.seen_revision {
                let _ = waiter.send(());
            } else {
                newer.push((revision, waiter));
            }
        }
        *pending = newer;
    }

    pub fn reject_trigger(&self, trigger: Trigger) {
        let mut registry = self.shared.registry.lock().unwrap();
        for (path, session) in &mut registry.sessions {
            let mut changed = false;
            for shortcut in &mut session.shortcuts {
                if shortcut.trigger == Some(trigger) {
                    shortcut.trigger = None;
                    changed = true;
                }
            }
            if changed {
                let _ = self.shared.tx.send(Event::Changed(path.clone()));
            }
        }
    }

    pub fn triggers(&self) -> Vec<Trigger> {
        self.shared
            .registry
            .lock()
            .unwrap()
            .sessions
            .values()
            .flat_map(|session| {
                session
                    .shortcuts
                    .iter()
                    .filter_map(|shortcut| shortcut.trigger)
            })
            .collect()
    }

    pub fn press(&self, keycode: u32, key: Keysym, mods: ModMask) -> bool {
        let mut registry = self.shared.registry.lock().unwrap();
        if registry.active.contains_key(&keycode) {
            return true;
        }
        let trigger = Trigger {
            key: key.for_binding(),
            mods,
        };
        if registry.reserved.contains(&trigger) {
            return false;
        }
        let activation = registry.sessions.iter().find_map(|(path, session)| {
            session
                .shortcuts
                .iter()
                .find(|shortcut| shortcut.trigger == Some(trigger))
                .map(|shortcut| Activation {
                    session: path.clone(),
                    id: shortcut.id.clone(),
                })
        });
        if let Some(activation) = activation {
            if !registry.active.values().any(|active| *active == activation) {
                self.shared.emit(activation.clone(), true);
            }
            registry.active.insert(keycode, activation);
            true
        } else {
            false
        }
    }

    pub fn release(&self, keycode: u32) -> bool {
        let mut registry = self.shared.registry.lock().unwrap();
        if let Some(activation) = registry.active.remove(&keycode) {
            if !registry.active.values().any(|active| *active == activation) {
                self.shared.emit(activation, false);
            }
            true
        } else {
            false
        }
    }

    pub fn suspend(&self) {
        self.shared
            .deactivate(&mut self.shared.registry.lock().unwrap());
    }
}

impl Drop for Shortcuts {
    fn drop(&mut self) {
        let paths: Vec<_> = self
            .shared
            .registry
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect();
        for path in paths {
            self.shared.remove_session(&path);
        }
        // Wake requests waiting for a native registration tick: shutdown will
        // not dispatch another frame, and those sessions have just closed.
        for (_, waiter) in self.shared.validation.lock().unwrap().drain(..) {
            let _ = waiter.send(());
        }
        let _ = self.shared.tx.send(Event::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

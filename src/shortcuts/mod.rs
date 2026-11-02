//! Application global shortcuts. Policy and portal sessions are shared by both
//! backends; only native key delivery and X11 passive grabs differ.
mod portal;
mod store;
#[cfg(test)]
mod tests;
mod ui;

use crate::types::{Keysym, ModMask, Modifier};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    generation: u64,
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
    revision: u64,
}

struct Shared {
    registry: Mutex<Registry>,
    revision: AtomicU64,
    tx: mpsc::Sender<Event>,
    wake: Mutex<Option<calloop::ping::Ping>>,
    epoch: Instant,
    store: Mutex<store::Store>,
    ui: Mutex<()>,
    validation_pending: AtomicBool,
    validation: Mutex<Vec<(u64, tokio::sync::oneshot::Sender<()>)>>,
    presenter: std::path::PathBuf,
}

impl Shared {
    fn changed(&self, registry: &mut Registry) {
        registry.revision = registry.revision.wrapping_add(1);
        self.revision.store(registry.revision, Ordering::Release);
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

    fn remove_session(&self, path: &str) {
        let dialog = {
            let mut registry = self.registry.lock().unwrap();
            let Some(session) = registry.sessions.remove(path) else {
                return;
            };
            self.changed(&mut registry);
            session.dialog
        };
        // Process creation and teardown share the dialog's child mutex. Never
        // wait for that mutex while holding the input registry.
        if let Some(dialog) = dialog {
            dialog.cancel();
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    activation: Activation,
    generation: u64,
}

/// Owned by the compositor thread. Portal workers only mutate the registry;
/// ordinary input performs an atomic revision check and a local hash lookup.
#[derive(Default)]
struct InputState {
    revision: u64,
    bindings: HashMap<Trigger, Binding>,
    active: HashMap<u32, (Trigger, Binding)>,
}

pub(crate) struct Shortcuts {
    shared: Arc<Shared>,
    rx: Option<mpsc::Receiver<Event>>,
    worker: Option<JoinHandle<()>>,
    seen_revision: u64,
    input: RefCell<InputState>,
}

impl Default for Shortcuts {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            shared: Arc::new(Shared {
                registry: Mutex::new(Registry::default()),
                revision: AtomicU64::new(0),
                tx,
                wake: Mutex::new(None),
                epoch: Instant::now(),
                store: Mutex::new(store::Store::default()),
                ui: Mutex::new(()),
                validation_pending: AtomicBool::new(false),
                validation: Mutex::new(Vec::new()),
                presenter: "instantmenu".into(),
            }),
            rx: Some(rx),
            worker: None,
            seen_revision: 0,
            input: RefCell::new(InputState::default()),
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
        // Application shortcuts are suspended outside default mode. Only
        // tables that can run in default mode need to reserve their chords.
        let reserved: HashSet<_> = bindings
            .keys
            .iter()
            .chain(&bindings.desktop_keybinds)
            .map(|key| Trigger {
                key: key.keysym.for_binding(),
                mods: key.mod_mask.cleaned(numlock),
            })
            .collect();
        let mut registry = self.shared.registry.lock().unwrap();
        if reserved != registry.reserved {
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
        drop(registry);
        self.reconcile();
        changed
    }

    pub fn take_changed(&self) -> bool {
        self.reconcile();
        self.seen_revision != self.shared.revision.load(Ordering::Acquire)
    }

    fn reconcile(&self) {
        if self.input.borrow().revision == self.shared.revision.load(Ordering::Acquire) {
            return;
        }
        let (revision, bindings) = {
            let registry = self.shared.registry.lock().unwrap();
            let bindings = registry
                .sessions
                .iter()
                .flat_map(|(path, session)| {
                    session.shortcuts.iter().filter_map(|shortcut| {
                        shortcut
                            .trigger
                            .filter(|trigger| !registry.reserved.contains(trigger))
                            .map(|trigger| {
                                (
                                    trigger,
                                    Binding {
                                        activation: Activation {
                                            session: path.clone(),
                                            id: shortcut.id.clone(),
                                        },
                                        generation: session.generation,
                                    },
                                )
                            })
                    })
                })
                .collect();
            (registry.revision, bindings)
        };
        let mut input = self.input.borrow_mut();
        input.bindings = bindings;
        let InputState {
            bindings, active, ..
        } = &mut *input;
        let mut stopped = HashSet::new();
        active.retain(|_, (trigger, binding)| {
            let keep = bindings.get(trigger) == Some(binding);
            if !keep {
                stopped.insert(binding.activation.clone());
            }
            keep
        });
        for activation in stopped {
            self.shared.emit(activation, false);
        }
        input.revision = revision;
    }

    /// Complete native registration before replying to a portal bind request.
    pub fn validated(&self) {
        if !self.shared.validation_pending.load(Ordering::Acquire) {
            return;
        }
        self.reconcile();
        let mut pending = self.shared.validation.lock().unwrap();
        let mut newer = Vec::new();
        for (revision, waiter) in pending.drain(..) {
            if revision <= self.seen_revision {
                let _ = waiter.send(());
            } else {
                newer.push((revision, waiter));
            }
        }
        self.shared
            .validation_pending
            .store(!newer.is_empty(), Ordering::Release);
        *pending = newer;
    }

    pub fn reject_trigger(&self, trigger: Trigger) {
        let mut registry = self.shared.registry.lock().unwrap();
        let mut any_changed = false;
        for (path, session) in &mut registry.sessions {
            let mut changed = false;
            for shortcut in &mut session.shortcuts {
                if shortcut.trigger == Some(trigger) {
                    shortcut.trigger = None;
                    changed = true;
                }
            }
            if changed {
                any_changed = true;
                let _ = self.shared.tx.send(Event::Changed(path.clone()));
            }
        }
        if any_changed {
            self.shared.changed(&mut registry);
        }
    }

    pub fn triggers(&self) -> Vec<Trigger> {
        self.reconcile();
        self.input.borrow().bindings.keys().copied().collect()
    }

    pub fn press(&self, keycode: u32, key: Keysym, mods: ModMask) -> bool {
        self.reconcile();
        let mut input = self.input.borrow_mut();
        if input.active.contains_key(&keycode) {
            return true;
        }
        let trigger = Trigger {
            key: key.for_binding(),
            mods,
        };
        let Some(binding) = input.bindings.get(&trigger).cloned() else {
            return false;
        };
        if !input
            .active
            .values()
            .any(|(_, active)| active.activation == binding.activation)
        {
            self.shared.emit(binding.activation.clone(), true);
        }
        input.active.insert(keycode, (trigger, binding));
        true
    }

    pub fn release(&self, keycode: u32) -> bool {
        self.reconcile();
        let mut input = self.input.borrow_mut();
        if let Some((_, binding)) = input.active.remove(&keycode) {
            if !input
                .active
                .values()
                .any(|(_, active)| active.activation == binding.activation)
            {
                self.shared.emit(binding.activation, false);
            }
            true
        } else {
            false
        }
    }

    pub fn has_active(&self) -> bool {
        !self.input.borrow().active.is_empty()
    }

    pub fn suspend(&self) {
        let activations: HashSet<_> = self
            .input
            .borrow_mut()
            .active
            .drain()
            .map(|(_, (_, binding))| binding.activation)
            .collect();
        for activation in activations {
            self.shared.emit(activation, false);
        }
    }
}

impl Drop for Shortcuts {
    fn drop(&mut self) {
        self.suspend();
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

use super::*;
use std::collections::HashMap;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{
    Connection,
    message::Header,
    object_server::{ObjectServer, SignalEmitter},
};

type Dict = HashMap<String, OwnedValue>;
type ShortcutsList = Vec<(String, Dict)>;
type Reply = (u32, Dict);
const PATH: &str = "/org/freedesktop/portal/desktop";
const NAME: &str = "org.freedesktop.impl.portal.desktop.instantwm";
const FRONTEND: &str = "org.freedesktop.portal.Desktop";
const INTERFACE: &str = "org.freedesktop.impl.portal.GlobalShortcuts";

fn invalid(message: impl Into<String>) -> zbus::fdo::Error {
    zbus::fdo::Error::InvalidArgs(message.into())
}
fn failed(message: impl Into<String>) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(message.into())
}
fn text(dict: &Dict, key: &str) -> Option<String> {
    dict.get(key)
        .and_then(|value| <&str>::try_from(value).ok())
        .map(str::to_owned)
}
fn value(text: &str) -> OwnedValue {
    Value::from(text).try_to_owned().unwrap()
}

/// Only the portal frontend may assert app identities or manage sessions.
async fn authorize(connection: &Connection, header: &Header<'_>) -> zbus::fdo::Result<String> {
    let sender = header.sender().ok_or_else(|| invalid("Missing sender"))?;
    let proxy = zbus::fdo::DBusProxy::new(connection).await?;
    let owner = proxy.get_name_owner(FRONTEND.try_into().unwrap()).await?;
    if sender.as_str() != owner.as_str() {
        return Err(zbus::fdo::Error::AccessDenied(
            "Only xdg-desktop-portal may call this backend".into(),
        ));
    }
    Ok(sender.to_string())
}

fn validate_path(path: &OwnedObjectPath, kind: &str) -> zbus::fdo::Result<()> {
    let prefix = format!("{PATH}/{kind}/");
    if !path.as_str().starts_with(&prefix) {
        return Err(invalid("Invalid portal object path"));
    }
    Ok(())
}

fn list(shortcuts: &[Shortcut]) -> ShortcutsList {
    shortcuts
        .iter()
        .filter_map(|shortcut| {
            shortcut.trigger.map(|trigger| {
                (
                    shortcut.id.clone(),
                    HashMap::from([
                        ("description".into(), value(&shortcut.description)),
                        ("trigger_description".into(), value(&trigger.text())),
                    ]),
                )
            })
        })
        .collect()
}

fn reply(shortcuts: &[Shortcut]) -> Reply {
    (
        0,
        HashMap::from([(
            "shortcuts".into(),
            Value::from(list(shortcuts)).try_to_owned().unwrap(),
        )]),
    )
}

fn parse_shortcuts(shortcuts: ShortcutsList) -> zbus::fdo::Result<Vec<Shortcut>> {
    if shortcuts.len() > 64 {
        return Err(invalid("At most 64 shortcuts per session"));
    }
    let mut ids = HashSet::new();
    shortcuts
        .into_iter()
        .map(|(id, properties)| {
            let description = text(&properties, "description")
                .ok_or_else(|| invalid("Missing shortcut description"))?;
            let preferred = text(&properties, "preferred_trigger");
            if id.is_empty()
                || id.len() > 256
                || description.is_empty()
                || description.len() > 1024
                || preferred.as_ref().is_some_and(|text| text.len() > 256)
                || !ids.insert(id.clone())
            {
                return Err(invalid("Invalid or duplicate shortcut"));
            }
            // A preferred trigger is a suggestion, never an implicit permission.
            let trigger = preferred
                .as_deref()
                .and_then(|text| Trigger::parse(text).ok());
            Ok(Shortcut {
                id,
                description,
                preferred,
                trigger,
            })
        })
        .collect()
}

struct Portal {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.GlobalShortcuts")]
impl Portal {
    #[zbus(property)]
    fn version(&self) -> u32 {
        2
    }

    // The portal wire signature plus zbus request context fixes this arity.
    #[allow(clippy::too_many_arguments)]
    async fn create_session(
        &self,
        handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        app_id: String,
        options: Dict,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<Reply> {
        let _ = options;
        let owner = authorize(connection, &header).await?;
        validate_path(&handle, "request")?;
        validate_path(&session_handle, "session")?;
        if app_id.len() > 256 {
            return Err(invalid("App ID too long"));
        }
        {
            let mut registry = self.shared.registry.lock().unwrap();
            if registry.sessions.len() >= 128
                || registry.sessions.contains_key(session_handle.as_str())
            {
                return Err(failed("Session already exists or session limit reached"));
            }
            registry.sessions.insert(
                session_handle.to_string(),
                Session {
                    app: app_id,
                    owner,
                    shortcuts: Vec::new(),
                    bound: false,
                    dialog: None,
                },
            );
        }
        let session = PortalSession {
            shared: self.shared.clone(),
            path: session_handle.to_string(),
        };
        match server.at(session_handle.clone(), session).await {
            Ok(true) => Ok((0, Dict::new())),
            result => {
                self.shared.remove_session(session_handle.as_str());
                Err(failed(format!("Could not export session: {result:?}")))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn bind_shortcuts(
        &self,
        handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        shortcuts: ShortcutsList,
        parent_window: String,
        options: Dict,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<Reply> {
        let _ = (parent_window, options);
        let owner = authorize(connection, &header).await?;
        let mut shortcuts = parse_shortcuts(shortcuts)?;
        let app = {
            let mut registry = self.shared.registry.lock().unwrap();
            let session = registry
                .sessions
                .get_mut(session_handle.as_str())
                .ok_or_else(|| invalid("Unknown session"))?;
            if session.owner != owner || session.bound {
                return Err(invalid("Session already bound or wrong owner"));
            }
            session.bound = true;
            session.app.clone()
        };
        let saved = self.shared.store.lock().unwrap().shortcuts(&app);
        let mut remembered = !app.is_empty();
        for shortcut in &mut shortcuts {
            if let Some(previous) = saved.iter().find(|previous| {
                previous.id == shortcut.id && previous.description == shortcut.description
            }) {
                shortcut.trigger = previous.trigger;
            } else {
                remembered = false;
            }
        }
        let result = self
            .configure(
                server,
                connection,
                handle,
                &session_handle,
                shortcuts,
                remembered,
            )
            .await;
        // On failure the session remains bound, as required by the public API.
        result
    }

    async fn list_shortcuts(
        &self,
        handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<Reply> {
        authorize(connection, &header).await?;
        validate_path(&handle, "request")?;
        let (app, bound, shortcuts) = {
            let registry = self.shared.registry.lock().unwrap();
            let session = registry
                .sessions
                .get(session_handle.as_str())
                .ok_or_else(|| invalid("Unknown session"))?;
            (
                session.app.clone(),
                session.bound,
                session.shortcuts.clone(),
            )
        };
        if bound {
            Ok(reply(&shortcuts))
        } else {
            Ok(reply(&self.shared.store.lock().unwrap().shortcuts(&app)))
        }
    }

    async fn configure_shortcuts(
        &self,
        session_handle: OwnedObjectPath,
        parent_window: String,
        options: Dict,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<()> {
        let _ = (parent_window, options);
        authorize(connection, &header).await?;
        let shortcuts = {
            let registry = self.shared.registry.lock().unwrap();
            let session = registry
                .sessions
                .get(session_handle.as_str())
                .ok_or_else(|| invalid("Unknown session"))?;
            if !session.bound {
                return Err(invalid("Session has not bound its shortcuts"));
            }
            session.shortcuts.clone()
        };
        // ConfigureShortcuts has no Request handle; session close cancels its UI.
        let handle = OwnedObjectPath::try_from(format!(
            "{}/configure",
            session_handle
                .as_str()
                .replacen("/session/", "/request/", 1)
        ))
        .unwrap();
        self.configure(
            server,
            connection,
            handle,
            &session_handle,
            shortcuts,
            false,
        )
        .await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn activated(
        emitter: &SignalEmitter<'_>,
        session_handle: OwnedObjectPath,
        shortcut_id: &str,
        timestamp: u64,
        options: Dict,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn deactivated(
        emitter: &SignalEmitter<'_>,
        session_handle: OwnedObjectPath,
        shortcut_id: &str,
        timestamp: u64,
        options: Dict,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn shortcuts_changed(
        emitter: &SignalEmitter<'_>,
        session_handle: OwnedObjectPath,
        shortcuts: ShortcutsList,
    ) -> zbus::Result<()>;
}

impl Portal {
    async fn configure(
        &self,
        server: &ObjectServer,
        connection: &Connection,
        handle: OwnedObjectPath,
        path: &OwnedObjectPath,
        shortcuts: Vec<Shortcut>,
        remembered: bool,
    ) -> zbus::fdo::Result<Reply> {
        validate_path(&handle, "request")?;
        let dialog = Arc::new(ui::Dialog::new(self.shared.presenter.clone()));
        let app = {
            let mut registry = self.shared.registry.lock().unwrap();
            let session = registry
                .sessions
                .get_mut(path.as_str())
                .ok_or_else(|| invalid("Session closed"))?;
            if session.dialog.is_some() {
                return Err(failed("Configuration already open"));
            }
            session.dialog = Some(dialog.clone());
            session.app.clone()
        };
        let request = PortalRequest {
            dialog: dialog.clone(),
        };
        let exported = server.at(handle.clone(), request).await;
        if !matches!(exported, Ok(true)) {
            if let Some(session) = self
                .shared
                .registry
                .lock()
                .unwrap()
                .sessions
                .get_mut(path.as_str())
            {
                session.dialog = None;
            }
            return Err(failed(format!("Could not export request: {exported:?}")));
        }
        let shared = self.shared.clone();
        let path_string = path.to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let dialog_for_thread = dialog.clone();
        std::thread::spawn(move || {
            let conflict = {
                let registry = shared.registry.lock().unwrap();
                shortcuts.iter().any(|shortcut| {
                    shortcut.trigger.is_some_and(|trigger| {
                        Shared::conflict(&registry, &path_string, &shortcut.id, trigger)
                    })
                })
            };
            let selected = if remembered && !conflict && !dialog_for_thread.cancelled() {
                Some(shortcuts)
            } else {
                ui::configure(&shared, &path_string, &app, shortcuts, &dialog_for_thread)
            };
            let _ = tx.send(selected);
        });
        let selected = rx.await.ok().flatten();
        let _ = server.remove::<PortalRequest, _>(handle).await;
        let (validation_tx, validation_rx) = tokio::sync::oneshot::channel();
        let accepted = {
            let mut registry = self.shared.registry.lock().unwrap();
            let valid = !dialog.cancelled() && registry.sessions.contains_key(path.as_str());
            if let Some(session) = registry.sessions.get_mut(path.as_str()) {
                session.dialog = None;
            }
            if let Some(shortcuts) = selected.filter(|_| valid) {
                let mut used = HashSet::new();
                if shortcuts.iter().any(|shortcut| {
                    shortcut.trigger.is_some_and(|trigger| {
                        Shared::conflict(&registry, path.as_str(), &shortcut.id, trigger)
                            || !used.insert(trigger)
                    })
                }) {
                    return Ok((2, Dict::new()));
                }
                self.shared.deactivate_session(&mut registry, path.as_str());
                let session = registry.sessions.get_mut(path.as_str()).unwrap();
                session.shortcuts = shortcuts.clone();
                let app = session.app.clone();
                self.shared.changed(&mut registry);
                self.shared
                    .validation
                    .lock()
                    .unwrap()
                    .push((registry.revision, validation_tx));
                Some((app, shortcuts, registry.revision))
            } else {
                None
            }
        };
        let Some((app, desired_shortcuts, generation)) = accepted else {
            return Ok((if dialog.failed() { 2 } else { 1 }, Dict::new()));
        };
        validation_rx
            .await
            .map_err(|_| failed("Compositor stopped"))?;
        let shortcuts = self
            .shared
            .registry
            .lock()
            .unwrap()
            .sessions
            .get(path.as_str())
            .map(|session| session.shortcuts.clone())
            .ok_or_else(|| invalid("Session closed"))?;
        // Persist on a worker: no filesystem access on the compositor thread or
        // the D-Bus executor. A write failure doesn't revoke the current grant.
        let shared = self.shared.clone();
        let (saved_tx, saved_rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            if let Err(error) =
                shared
                    .store
                    .lock()
                    .unwrap()
                    .save(&app, &desired_shortcuts, generation)
            {
                log::warn!("global shortcuts: could not save preferences: {error}");
            }
            let _ = saved_tx.send(());
        });
        let _ = saved_rx.await;
        let emitter = SignalEmitter::new(connection, PATH).map_err(|e| failed(e.to_string()))?;
        Self::shortcuts_changed(&emitter, path.clone(), list(&shortcuts))
            .await
            .map_err(|e| failed(e.to_string()))?;
        Ok(reply(&shortcuts))
    }
}

struct PortalSession {
    shared: Arc<Shared>,
    path: String,
}
#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl PortalSession {
    #[zbus(property)]
    fn version(&self) -> u32 {
        1
    }
    async fn close(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<()> {
        authorize(connection, &header).await?;
        self.shared.remove_session(&self.path);
        server
            .remove::<Self, _>(self.path.as_str())
            .await
            .map_err(|e| failed(e.to_string()))?;
        Ok(())
    }
    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

struct PortalRequest {
    dialog: Arc<ui::Dialog>,
}
#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl PortalRequest {
    async fn close(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        authorize(connection, &header).await?;
        self.dialog.cancel();
        Ok(())
    }
}

pub(super) fn serve(
    shared: Arc<Shared>,
    rx: mpsc::Receiver<Event>,
    address: Option<String>,
    ready: mpsc::SyncSender<Result<(), String>>,
) -> zbus::Result<()> {
    if address.is_none() {
        *shared.store.lock().unwrap() = store::Store::load();
    }
    let builder = if let Some(address) = address.as_deref() {
        zbus::blocking::connection::Builder::address(address)?
    } else {
        zbus::blocking::connection::Builder::session()?
    };
    let connection = builder
        // Refuse to steal a host compositor's portal name in nested sessions.
        .name(NAME)?
        .serve_at(
            PATH,
            Portal {
                shared: shared.clone(),
            },
        )?
        .build()?;
    log::info!("global shortcuts portal ready");
    // Watch the frontend's unique owner. It closes sessions on app disconnect;
    // if the frontend itself exits, discard all of its sessions as well.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .add_arg(FRONTEND)?
        .build();
    let mut watcher = zbus::blocking::MessageIterator::for_match_rule(rule, &connection, Some(16))?;
    let watch_shared = Arc::downgrade(&shared);
    let watch_connection = connection.clone();
    let watcher_thread = std::thread::spawn(move || {
        for message in &mut watcher {
            let Ok(message) = message else {
                if let Some(shared) = watch_shared.upgrade() {
                    let _ = shared.tx.send(Event::BusGone);
                }
                break;
            };
            let Ok((_, old, _)) = message.body().deserialize::<(String, String, String)>() else {
                continue;
            };
            if old.is_empty() {
                continue;
            }
            let Some(shared) = watch_shared.upgrade() else {
                break;
            };
            let paths: Vec<_> = shared
                .registry
                .lock()
                .unwrap()
                .sessions
                .iter()
                .filter(|(_, session)| session.owner == old)
                .map(|(path, _)| path.clone())
                .collect();
            for path in paths {
                shared.remove_session(&path);
                let _ = watch_connection
                    .object_server()
                    .remove::<PortalSession, _>(path);
            }
        }
    });
    let _ = ready.send(Ok(()));
    let mut result = Ok(());
    while let Ok(event) = rx.recv() {
        match event {
            Event::Stop | Event::BusGone => break,
            Event::Changed(path) => {
                let shortcuts = shared
                    .registry
                    .lock()
                    .unwrap()
                    .sessions
                    .get(&path)
                    .map(|session| list(&session.shortcuts));
                if let Some(shortcuts) = shortcuts {
                    let path = OwnedObjectPath::try_from(path).unwrap();
                    let _ = connection.emit_signal(
                        None::<&str>,
                        PATH,
                        INTERFACE,
                        "ShortcutsChanged",
                        &(path, shortcuts),
                    );
                }
            }
            Event::Key {
                activation,
                pressed,
                time,
            } => {
                let path = OwnedObjectPath::try_from(activation.session).unwrap();
                if let Err(error) = connection.emit_signal(
                    None::<&str>,
                    PATH,
                    INTERFACE,
                    if pressed { "Activated" } else { "Deactivated" },
                    &(path, activation.id, time, Dict::new()),
                ) {
                    result = Err(error);
                    break;
                }
            }
        }
    }
    let _ = connection.close();
    let _ = watcher_thread.join();
    result
}

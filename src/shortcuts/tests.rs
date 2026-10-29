use super::*;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

type Dict = HashMap<String, OwnedValue>;
type List = Vec<(String, Dict)>;
type Reply = (u32, Dict);
const NAME: &str = "org.freedesktop.impl.portal.desktop.instantwm";
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.impl.portal.GlobalShortcuts";

fn shortcut(id: &str, trigger: &str) -> Shortcut {
    Shortcut {
        id: id.into(),
        description: format!("Action {id}"),
        preferred: None,
        trigger: Some(Trigger::parse(trigger).unwrap()),
    }
}
fn insert(shortcuts: &Shortcuts, path: &str, actions: Vec<Shortcut>) {
    shortcuts.shared.registry.lock().unwrap().sessions.insert(
        path.into(),
        Session {
            app: "test.app".into(),
            owner: "owner".into(),
            shortcuts: actions,
            bound: true,
            dialog: None,
        },
    );
}

#[test]
fn xdg_triggers_round_trip_and_reserve_emergency_keys() {
    for text in [
        "CTRL+ALT+space",
        "LOGO+SHIFT+F12",
        "XF86AudioMute",
        "CTRL+plus",
        "ALT+Return",
    ] {
        let trigger = Trigger::parse(text).unwrap();
        assert_eq!(Trigger::parse(&trigger.text()).unwrap(), trigger);
    }
    assert_eq!(
        Trigger::parse("CTRL+A").unwrap(),
        Trigger::parse("CTRL+a").unwrap()
    );
    for text in [
        "",
        "CTRL+",
        "CTRL+CTRL+a",
        "META+a",
        "CTRL+ALT+F1",
        "LOGO+SHIFT+Escape",
        "Control_L",
        "Shift_L",
        "caps_lock",
        "XF86Switch_VT_1",
        "CTRL+not_a_keysym",
        "CTRL+a;echo",
    ] {
        assert!(Trigger::parse(text).is_err(), "accepted {text}");
    }
}

#[test]
fn activation_is_paired_by_physical_key_and_repeat_is_ignored() {
    let shortcuts = Shortcuts::default();
    insert(&shortcuts, "session", vec![shortcut("talk", "CTRL+space")]);
    let trigger = Trigger::parse("CTRL+space").unwrap();
    assert!(shortcuts.press(65, trigger.key, trigger.mods));
    assert!(shortcuts.press(65, trigger.key, trigger.mods));
    assert!(!shortcuts.release(66));
    // Releases don't depend on the current layout, modifiers or focus.
    assert!(shortcuts.release(65));
    assert!(!shortcuts.release(65));
    let rx = shortcuts.rx.as_ref().unwrap();
    assert!(matches!(
        rx.try_recv().unwrap(),
        Event::Key { pressed: true, .. }
    ));
    assert!(matches!(
        rx.try_recv().unwrap(),
        Event::Key { pressed: false, .. }
    ));
    assert!(rx.try_recv().is_err());
}

#[test]
fn closing_sessions_and_locking_stop_held_actions() {
    let shortcuts = Shortcuts::default();
    insert(&shortcuts, "first", vec![shortcut("talk", "CTRL+space")]);
    insert(&shortcuts, "second", vec![shortcut("other", "ALT+space")]);
    assert!(shortcuts.press(
        65,
        Keysym::from_name("space").unwrap(),
        ModMask::from_modifier(Modifier::Control)
    ));
    assert!(shortcuts.press(
        66,
        Keysym::from_name("space").unwrap(),
        ModMask::from_modifier(Modifier::Alt)
    ));
    shortcuts.shared.remove_session("first");
    assert!(!shortcuts.release(65));
    assert_eq!(shortcuts.shared.registry.lock().unwrap().active.len(), 1);
    shortcuts.suspend();
    assert!(!shortcuts.release(66));
    assert!(shortcuts.shared.registry.lock().unwrap().active.is_empty());
    let events: Vec<_> = shortcuts.rx.as_ref().unwrap().try_iter().collect();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Key { pressed: true, .. }))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Key { pressed: false, .. }))
            .count(),
        2
    );
}

#[test]
fn configuration_reload_withdraws_conflicting_portal_bindings() {
    let mut shortcuts = Shortcuts::default();
    let trigger = Trigger::parse("CTRL+space").unwrap();
    insert(&shortcuts, "session", vec![shortcut("talk", "CTRL+space")]);
    assert!(shortcuts.press(65, trigger.key, trigger.mods));
    let mut bindings = crate::core_state::BindingConfig::default();
    bindings.keys.push(crate::types::Key {
        keysym: trigger.key,
        mod_mask: trigger.mods,
        action: crate::actions::KeyAction::named(crate::actions::NamedAction::Quit),
        origin: crate::types::KeybindOrigin::CompiledDefault,
    });
    assert!(shortcuts.sync(&bindings, ModMask::NONE));
    assert!(shortcuts.triggers().is_empty());
    assert!(!shortcuts.press(65, trigger.key, trigger.mods));
    assert!(!shortcuts.sync(&bindings, ModMask::NONE));
    let events: Vec<_> = shortcuts.rx.as_ref().unwrap().try_iter().collect();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Changed(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Key { pressed: false, .. }))
    );
}

#[test]
fn preferences_round_trip_atomically_without_app_id_path_components() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("global-shortcuts.toml");
    let mut store = store::Store::with_path(path.clone());
    let actions = vec![shortcut("talk", "CTRL+space")];
    store.save("../../strange.app", &actions, 1).unwrap();
    store.save("", &actions, 1).unwrap();
    let saved: store::Store = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        saved.shortcuts("../../strange.app")[0].trigger,
        actions[0].trigger
    );
    assert!(saved.shortcuts("").is_empty());
    assert!(saved.shortcuts("other.app").is_empty());
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

struct Bus {
    child: Child,
    address: String,
}
impl Bus {
    fn new() -> Self {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon is required for portal tests");
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self {
            child,
            address: address.trim().into(),
        }
    }
    fn connection(&self) -> Connection {
        zbus::blocking::connection::Builder::address(self.address.as_str())
            .unwrap()
            .build()
            .unwrap()
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Harness {
    shortcuts: Shortcuts,
    frontend: Connection,
    bus: Bus,
    dir: tempfile::TempDir,
}
impl Harness {
    fn new() -> Self {
        let bus = Bus::new();
        let frontend = bus.connection();
        frontend
            .request_name("org.freedesktop.portal.Desktop")
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let presenter = dir.path().join("presenter");
        std::fs::write(&presenter, "#!/bin/sh\ncat >/dev/null\nprintf 'allow\\n'\n").unwrap();
        std::fs::set_permissions(&presenter, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut shortcuts = Shortcuts::default();
        Arc::get_mut(&mut shortcuts.shared).unwrap().presenter = presenter;
        *shortcuts.shared.store.lock().unwrap() =
            store::Store::with_path(dir.path().join("saved.toml"));
        let shared = shortcuts.shared.clone();
        let rx = shortcuts.rx.take().unwrap();
        let address = bus.address.clone();
        let (tx, ready) = mpsc::sync_channel(1);
        shortcuts.worker = Some(std::thread::spawn(move || {
            portal::serve(shared, rx, Some(address), tx).unwrap()
        }));
        ready.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        Self {
            shortcuts,
            frontend,
            bus,
            dir,
        }
    }
    fn path(token: &str) -> OwnedObjectPath {
        format!("{PATH}/session/test/{token}").try_into().unwrap()
    }
    fn request(token: &str) -> OwnedObjectPath {
        format!("{PATH}/request/test/{token}").try_into().unwrap()
    }
    fn create(&self, token: &str) {
        let proxy = Proxy::new(&self.frontend, NAME, PATH, IFACE).unwrap();
        let result: Reply = proxy
            .call(
                "CreateSession",
                &(
                    Self::request(token),
                    Self::path(token),
                    "test.app",
                    Dict::new(),
                ),
            )
            .unwrap();
        assert_eq!(result.0, 0);
    }
    fn drive<T: Send + 'static>(
        &mut self,
        call: impl FnOnce(Connection) -> T + Send + 'static,
    ) -> T {
        let conn = self.frontend.clone();
        let (tx, rx) = mpsc::channel();
        let task = std::thread::spawn(move || {
            let _ = tx.send(call(conn));
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.shortcuts
                .sync(&crate::core_state::BindingConfig::default(), ModMask::NONE);
            self.shortcuts.validated();
            match rx.recv_timeout(Duration::from_millis(5)) {
                Ok(result) => {
                    task.join().unwrap();
                    return result;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    task.join().unwrap();
                    panic!("portal call task failed");
                }
                Err(_) => assert!(Instant::now() < deadline, "portal call timed out"),
            }
        }
    }
    fn bind(&mut self, token: &'static str, actions: List) -> zbus::Result<Reply> {
        self.drive(move |conn| {
            Proxy::new(&conn, NAME, PATH, IFACE).unwrap().call(
                "BindShortcuts",
                &(
                    Self::request(token),
                    Self::path(token),
                    actions,
                    "",
                    Dict::new(),
                ),
            )
        })
    }
    fn list(&self, token: &str) -> List {
        let reply: Reply = Proxy::new(&self.frontend, NAME, PATH, IFACE)
            .unwrap()
            .call("ListShortcuts", &(Self::request(token), Self::path(token)))
            .unwrap();
        assert_eq!(reply.0, 0);
        reply
            .1
            .get("shortcuts")
            .unwrap()
            .try_clone()
            .unwrap()
            .try_into()
            .unwrap()
    }
    fn close(&self, token: &str) {
        let path = Self::path(token);
        let proxy = Proxy::new(
            &self.frontend,
            NAME,
            path.as_str(),
            "org.freedesktop.impl.portal.Session",
        )
        .unwrap();
        proxy.call::<_, _, ()>("Close", &()).unwrap();
    }
}
fn actions() -> List {
    vec![(
        "talk".into(),
        HashMap::from([
            (
                "description".into(),
                Value::from("Start voice input").try_to_owned().unwrap(),
            ),
            (
                "preferred_trigger".into(),
                Value::from("CTRL+ALT+space").try_to_owned().unwrap(),
            ),
        ]),
    )]
}

#[test]
fn portal_dbus_bind_list_reconfigure_and_session_cleanup() {
    let mut harness = Harness::new();
    harness.create("one");
    assert!(harness.list("one").is_empty());
    assert_eq!(harness.bind("one", actions()).unwrap().0, 0);
    assert_eq!(harness.list("one")[0].0, "talk");
    assert!(harness.bind("one", actions()).is_err());
    let result: zbus::Result<()> = harness.drive(|conn| {
        Proxy::new(&conn, NAME, PATH, IFACE).unwrap().call(
            "ConfigureShortcuts",
            &(Harness::path("one"), "", Dict::new()),
        )
    });
    result.unwrap();
    let trigger = Trigger::parse("CTRL+ALT+space").unwrap();
    assert!(harness.shortcuts.press(65, trigger.key, trigger.mods));
    harness.close("one");
    assert!(harness.shortcuts.triggers().is_empty());
    assert!(!harness.shortcuts.release(65));
}

#[test]
fn portal_rejects_impersonation_duplicate_ids_and_foreign_paths() {
    let mut harness = Harness::new();
    let outsider = harness.bus.connection();
    let proxy = Proxy::new(&outsider, NAME, PATH, IFACE).unwrap();
    let response: zbus::Result<Reply> = proxy.call(
        "CreateSession",
        &(
            Harness::request("evil"),
            Harness::path("evil"),
            "trusted.app",
            Dict::new(),
        ),
    );
    assert!(response.is_err());
    harness.create("one");
    let mut duplicate = actions();
    duplicate.extend(actions());
    assert!(harness.bind("one", duplicate).is_err());
    // Validation errors do not consume the one permitted BindShortcuts call.
    assert_eq!(harness.bind("one", actions()).unwrap().0, 0);
    let proxy = Proxy::new(&harness.frontend, NAME, PATH, IFACE).unwrap();
    let bad_path = OwnedObjectPath::try_from("/arbitrary/path").unwrap();
    let response: zbus::Result<Reply> = proxy.call(
        "CreateSession",
        &(Harness::request("two"), bad_path, "test.app", Dict::new()),
    );
    assert!(response.is_err());
}

#[test]
fn portal_frontend_restart_removes_its_sessions() {
    let mut harness = Harness::new();
    harness.create("one");
    assert_eq!(harness.bind("one", actions()).unwrap().0, 0);
    harness
        .frontend
        .release_name("org.freedesktop.portal.Desktop")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !harness
        .shortcuts
        .shared
        .registry
        .lock()
        .unwrap()
        .sessions
        .is_empty()
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(harness.shortcuts.triggers().is_empty());
}

#[test]
fn portal_remembers_only_user_approved_bindings() {
    let mut harness = Harness::new();
    harness.create("one");
    harness.bind("one", actions()).unwrap();
    // Wait for the asynchronous atomic save, then remove the presenter. A
    // remembered grant must work without opening a second approval dialog.
    let deadline = Instant::now() + Duration::from_secs(3);
    while harness
        .shortcuts
        .shared
        .store
        .lock()
        .unwrap()
        .shortcuts("test.app")
        .is_empty()
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.close("one");
    std::fs::remove_file(harness.dir.path().join("presenter")).unwrap();
    harness.create("two");
    assert_eq!(harness.list("two").len(), 1);
    assert_eq!(harness.bind("two", actions()).unwrap().0, 0);
}

#[test]
fn portal_request_close_cancels_the_dialog_and_does_not_grant_keys() {
    let harness = Harness::new();
    std::fs::write(
        harness.dir.path().join("presenter"),
        "#!/bin/sh\nexec sleep 30\n",
    )
    .unwrap();
    harness.create("one");
    let connection = harness.frontend.clone();
    let (tx, rx) = mpsc::channel();
    let task = std::thread::spawn(move || {
        let result: zbus::Result<Reply> = Proxy::new(&connection, NAME, PATH, IFACE).unwrap().call(
            "BindShortcuts",
            &(
                Harness::request("one"),
                Harness::path("one"),
                actions(),
                "",
                Dict::new(),
            ),
        );
        tx.send(result).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if harness
            .shortcuts
            .shared
            .registry
            .lock()
            .unwrap()
            .sessions
            .get(Harness::path("one").as_str())
            .unwrap()
            .dialog
            .is_some()
        {
            let request = Harness::request("one");
            let proxy = Proxy::new(
                &harness.frontend,
                NAME,
                request.as_str(),
                "org.freedesktop.impl.portal.Request",
            )
            .unwrap();
            if proxy.call::<_, _, ()>("Close", &()).is_ok() {
                break;
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(3)).unwrap().unwrap().0,
        1
    );
    task.join().unwrap();
    assert!(harness.shortcuts.triggers().is_empty());
}

#[test]
fn two_physical_keys_for_one_action_have_one_activation_pair() {
    let shortcuts = Shortcuts::default();
    insert(&shortcuts, "session", vec![shortcut("talk", "CTRL+space")]);
    let trigger = Trigger::parse("CTRL+space").unwrap();
    assert!(shortcuts.press(65, trigger.key, trigger.mods));
    assert!(shortcuts.press(66, trigger.key, trigger.mods));
    assert!(shortcuts.release(65));
    assert_eq!(shortcuts.shared.registry.lock().unwrap().active.len(), 1);
    assert!(shortcuts.release(66));
    let events: Vec<_> = shortcuts.rx.as_ref().unwrap().try_iter().collect();
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], Event::Key { pressed: true, .. }));
    assert!(matches!(events[1], Event::Key { pressed: false, .. }));
}

#[test]
fn older_save_cannot_overwrite_a_newer_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = store::Store::with_path(dir.path().join("saved.toml"));
    store
        .save("test.app", &[shortcut("talk", "ALT+space")], 20)
        .unwrap();
    store
        .save("test.app", &[shortcut("talk", "CTRL+space")], 10)
        .unwrap();
    assert_eq!(
        store.shortcuts("test.app")[0].trigger,
        Some(Trigger::parse("ALT+space").unwrap())
    );
}

#[test]
fn native_registration_acknowledgement_cannot_race_a_later_binding() {
    let mut shortcuts = Shortcuts::default();
    shortcuts.sync(&crate::core_state::BindingConfig::default(), ModMask::NONE);
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    {
        let mut registry = shortcuts.shared.registry.lock().unwrap();
        shortcuts.shared.changed(&mut registry);
        shortcuts
            .shared
            .validation
            .lock()
            .unwrap()
            .push((registry.revision, tx));
    }
    shortcuts.validated();
    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    shortcuts.sync(&crate::core_state::BindingConfig::default(), ModMask::NONE);
    shortcuts.validated();
    assert_eq!(rx.try_recv(), Ok(()));
}

impl Shortcuts {
    pub(crate) fn register_test_shortcut(&self, trigger: &str) {
        insert(self, "test", vec![shortcut("test", trigger)]);
    }
    pub(crate) fn test_key_events(&self) -> Vec<bool> {
        self.rx
            .as_ref()
            .unwrap()
            .try_iter()
            .filter_map(|event| match event {
                Event::Key { pressed, .. } => Some(pressed),
                _ => None,
            })
            .collect()
    }
}

#[test]
#[ignore = "requires a dedicated Xvfb display"]
fn x11_native_shortcut_grabs_release_pairing_and_conflict_detection() {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::{
        Event as XEvent,
        xproto::{self, ConnectionExt as _},
        xtest,
    };
    let (conn, screen) = x11rb::connect(None).unwrap();
    let root = conn.setup().roots[screen].root;
    let mut wm = crate::wm::X11Wm::new(crate::backend::X11BackendData::new(conn, screen));
    wm.backend.x11_runtime.root = root;
    assert!(crate::backend::x11::keyboard::enable_detectable_repeat(
        &wm.backend.conn
    ));
    {
        let ctx = wm.x11_ctx();
        let crate::contexts::WmCtx::X11(ctx) = ctx else {
            unreachable!()
        };
        assert!(crate::backend::x11::keyboard::refresh_keyboard_mapping(
            &ctx.x11,
            ctx.x11_runtime
        ));
    }
    wm.core
        .state
        .shortcuts
        .register_test_shortcut("CTRL+ALT+space");
    wm.backend
        .conn
        .change_window_attributes(
            root,
            &xproto::ChangeWindowAttributesAux::new()
                .event_mask(xproto::EventMask::KEY_PRESS | xproto::EventMask::KEY_RELEASE),
        )
        .unwrap()
        .check()
        .unwrap();
    let codes = |name: &str| {
        let symbol = Keysym::from_name(name).unwrap();
        (wm.backend.conn.setup().min_keycode..=wm.backend.conn.setup().max_keycode)
            .find(|code| wm.backend.x11_runtime.keyboard_mapping.keysym(*code, 0) == symbol)
            .unwrap()
    };
    let (ctrl, alt, space) = (codes("Control_L"), codes("Alt_L"), codes("space"));
    wm.x11_ctx().refresh_key_grabs();
    let (injector, _) = x11rb::connect(None).unwrap();
    for (kind, code) in [
        (xproto::KEY_PRESS_EVENT, ctrl),
        (xproto::KEY_PRESS_EVENT, alt),
        (xproto::KEY_PRESS_EVENT, space),
        (xproto::KEY_RELEASE_EVENT, ctrl),
        (xproto::KEY_RELEASE_EVENT, alt),
        (xproto::KEY_RELEASE_EVENT, space),
    ] {
        xtest::fake_input(&injector, kind, code, 0, root, 0, 0, 0)
            .unwrap()
            .check()
            .unwrap();
    }
    injector.get_input_focus().unwrap().reply().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut released = false;
    while !released {
        match wm.backend.conn.poll_for_event().unwrap() {
            Some(XEvent::KeyPress(event)) => {
                let crate::contexts::WmCtx::X11(mut ctx) = wm.x11_ctx() else {
                    unreachable!()
                };
                crate::backend::x11::keyboard::key_press(&mut ctx, &event);
            }
            Some(XEvent::KeyRelease(event)) => {
                wm.core.state.shortcuts.release(u32::from(event.detail));
                released |= event.detail == space;
            }
            Some(_) => {}
            None => {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
    assert_eq!(wm.core.state.shortcuts.test_key_events(), vec![true, false]);
    wm.backend
        .conn
        .ungrab_key(0, root, xproto::ModMask::ANY)
        .unwrap()
        .check()
        .unwrap();
    injector
        .grab_key(
            false,
            root,
            xproto::ModMask::CONTROL | xproto::ModMask::M1,
            space,
            xproto::GrabMode::ASYNC,
            xproto::GrabMode::ASYNC,
        )
        .unwrap()
        .check()
        .unwrap();
    wm.x11_ctx().refresh_key_grabs();
    assert!(wm.core.state.shortcuts.triggers().is_empty());
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires /usr/lib/xdg-desktop-portal"]
fn real_portal_frontend_discovers_backend_and_forwards_shortcuts() {
    let mut harness = Harness::new();
    harness
        .frontend
        .release_name("org.freedesktop.portal.Desktop")
        .unwrap();
    let share = harness.dir.path().join("share");
    let config = harness.dir.path().join("config");
    std::fs::create_dir_all(share.join("xdg-desktop-portal/portals")).unwrap();
    std::fs::create_dir_all(config.join("xdg-desktop-portal")).unwrap();
    std::fs::write(
        share.join("xdg-desktop-portal/portals/instantwm.portal"),
        include_str!("../../resources/instantwm.portal"),
    )
    .unwrap();
    std::fs::write(
        config.join("xdg-desktop-portal/instantwm-portals.conf"),
        "[preferred]\ndefault=instantwm\n",
    )
    .unwrap();
    std::fs::create_dir_all(share.join("applications")).unwrap();
    std::fs::write(
        share.join("applications/org.instantwm.PortalTest.desktop"),
        "[Desktop Entry]\nType=Application\nName=Portal Test\nExec=true\n",
    )
    .unwrap();
    // XDG_DESKTOP_PORTAL_DIR also redirects configuration lookup in the
    // installed frontend. Exercise the configuration shipped by the WM.
    std::fs::write(
        share.join("xdg-desktop-portal/portals/instantwm-portals.conf"),
        include_str!("../../resources/instantwm-portals.conf"),
    )
    .unwrap();
    let log = std::fs::File::create(harness.dir.path().join("frontend.log")).unwrap();
    let _frontend = ChildGuard(
        Command::new("/usr/lib/xdg-desktop-portal")
            .arg("--verbose")
            .env("DBUS_SESSION_BUS_ADDRESS", &harness.bus.address)
            .env("XDG_DESKTOP_PORTAL_TEST_APP_INFO_KIND", "host")
            .env(
                "XDG_DESKTOP_PORTAL_TEST_HOST_APPID",
                "org.instantwm.PortalTest",
            )
            .env(
                "XDG_DESKTOP_PORTAL_DIR",
                share.join("xdg-desktop-portal/portals"),
            )
            .env("GSETTINGS_SCHEMA_DIR", "/usr/share/glib-2.0/schemas")
            .env("XDG_DATA_HOME", &share)
            .env("XDG_DATA_DIRS", &share)
            .env("XDG_CONFIG_HOME", config)
            .env("XDG_CURRENT_DESKTOP", "instantwm")
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    let dbus = zbus::blocking::fdo::DBusProxy::new(&harness.frontend).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while dbus
        .get_name_owner("org.freedesktop.portal.Desktop".try_into().unwrap())
        .is_err()
    {
        assert!(
            Instant::now() < deadline,
            "{}",
            std::fs::read_to_string(harness.dir.path().join("frontend.log")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let result = harness.drive(|conn| -> zbus::Result<(u32, Dict)> {
        let proxy = Proxy::new(
            &conn,
            "org.freedesktop.portal.Desktop",
            PATH,
            "org.freedesktop.portal.GlobalShortcuts",
        )?;
        assert!(proxy.get_property::<u32>("version")? >= 1);
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.portal.Request")?
            .member("Response")?
            .build();
        let mut responses = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(8))?;
        let options = Dict::from([
            (
                "handle_token".into(),
                Value::from("create").try_to_owned().unwrap(),
            ),
            (
                "session_handle_token".into(),
                Value::from("session").try_to_owned().unwrap(),
            ),
        ]);
        let _: OwnedObjectPath = proxy.call("CreateSession", &(options,))?;
        let response: Reply = responses.next().unwrap()?.body().deserialize()?;
        assert_eq!(response.0, 0);
        let session = <&str>::try_from(response.1.get("session_handle").unwrap()).unwrap();
        let session = OwnedObjectPath::try_from(session).unwrap();
        let _: OwnedObjectPath =
            proxy.call("BindShortcuts", &(session, actions(), "", Dict::new()))?;
        responses.next().unwrap()?.body().deserialize()
    });
    assert_eq!(
        result
            .unwrap_or_else(|error| panic!(
                "{error}\n{}",
                std::fs::read_to_string(harness.dir.path().join("frontend.log")).unwrap()
            ))
            .0,
        0,
        "{}",
        std::fs::read_to_string(harness.dir.path().join("frontend.log")).unwrap()
    );
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.portal.GlobalShortcuts")
        .unwrap()
        .build();
    let mut signals =
        zbus::blocking::MessageIterator::for_match_rule(rule, &harness.frontend, Some(8)).unwrap();
    let trigger = Trigger::parse("CTRL+ALT+space").unwrap();
    assert!(harness.shortcuts.press(65, trigger.key, trigger.mods));
    assert!(harness.shortcuts.release(65));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let names: Vec<_> = (0..2)
            .map(|_| {
                signals
                    .next()
                    .unwrap()
                    .unwrap()
                    .header()
                    .member()
                    .unwrap()
                    .to_string()
            })
            .collect();
        let _ = tx.send(names);
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(3)).unwrap(),
        vec!["Activated", "Deactivated"]
    );
}

#[test]
fn portal_user_can_edit_and_disable_an_application_shortcut() {
    let mut harness = Harness::new();
    let presenter = harness.dir.path().join("presenter");
    std::fs::write(
        &presenter,
        r##"#!/bin/sh
cat >/dev/null
case "$*" in
    *--input-only*) printf 'CTRL+SHIFT+F9\n';;
    *) if test -e "$0.state"; then printf 'allow\n'; else touch "$0.state"; printf 'edit0\n'; fi;;
esac
"##,
    )
    .unwrap();
    harness.create("one");
    assert_eq!(harness.bind("one", actions()).unwrap().0, 0);
    let bound = harness.list("one");
    assert_eq!(
        <&str>::try_from(bound[0].1.get("trigger_description").unwrap()).unwrap(),
        "CTRL+SHIFT+F9"
    );
    std::fs::remove_file(presenter.with_extension("state")).unwrap();
    let script = std::fs::read_to_string(&presenter)
        .unwrap()
        .replace("CTRL+SHIFT+F9", "");
    std::fs::write(&presenter, script).unwrap();
    let response: zbus::Result<()> = harness.drive(|conn| {
        Proxy::new(&conn, NAME, PATH, IFACE).unwrap().call(
            "ConfigureShortcuts",
            &(Harness::path("one"), "", Dict::new()),
        )
    });
    response.unwrap();
    assert!(harness.list("one").is_empty());
    assert!(harness.shortcuts.triggers().is_empty());
}

#[test]
fn closing_a_session_cancels_its_queued_dialog_promptly() {
    let harness = Harness::new();
    std::fs::write(
        harness.dir.path().join("presenter"),
        "#!/bin/sh\nexec sleep 30\n",
    )
    .unwrap();
    harness.create("one");
    harness.create("two");
    let call = |connection: Connection, token: &'static str| {
        let (tx, rx) = mpsc::channel();
        let task = std::thread::spawn(move || {
            let result: zbus::Result<Reply> =
                Proxy::new(&connection, NAME, PATH, IFACE).unwrap().call(
                    "BindShortcuts",
                    &(
                        Harness::request(token),
                        Harness::path(token),
                        actions(),
                        "",
                        Dict::new(),
                    ),
                );
            let _ = tx.send(result);
        });
        (rx, task)
    };
    let wait_dialog = |token: &str| {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let open = harness
                .shortcuts
                .shared
                .registry
                .lock()
                .unwrap()
                .sessions
                .get(Harness::path(token).as_str())
                .is_some_and(|session| session.dialog.is_some());
            if open {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let (first, first_task) = call(harness.frontend.clone(), "one");
    wait_dialog("one");
    let deadline = Instant::now() + Duration::from_secs(3);
    while harness.shortcuts.shared.ui.try_lock().is_ok() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let (second, second_task) = call(harness.frontend.clone(), "two");
    wait_dialog("two");
    harness.close("two");
    assert_eq!(
        second
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .0,
        1
    );
    second_task.join().unwrap();
    // The first menu is still open: cancelling the queued request did not
    // need to wait for or dismiss an unrelated application's approval prompt.
    assert!(first.try_recv().is_err());
    harness.close("one");
    assert_eq!(
        first
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .0,
        1
    );
    first_task.join().unwrap();
}

#[test]
fn a_missing_presenter_is_an_error_and_never_grants_shortcuts() {
    let mut harness = Harness::new();
    std::fs::remove_file(harness.dir.path().join("presenter")).unwrap();
    harness.create("one");
    assert_eq!(harness.bind("one", actions()).unwrap().0, 2);
    assert!(harness.shortcuts.triggers().is_empty());
}

#[test]
fn compositor_shutdown_completes_while_a_request_waits_for_native_registration() {
    let harness = Harness::new();
    harness.create("one");
    let connection = harness.frontend.clone();
    let call = std::thread::spawn(move || {
        let response: zbus::Result<Reply> =
            Proxy::new(&connection, NAME, PATH, IFACE).unwrap().call(
                "BindShortcuts",
                &(
                    Harness::request("one"),
                    Harness::path("one"),
                    actions(),
                    "",
                    Dict::new(),
                ),
            );
        response
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while harness
        .shortcuts
        .shared
        .validation
        .lock()
        .unwrap()
        .is_empty()
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let Harness {
        shortcuts,
        frontend: _frontend,
        bus: _bus,
        dir: _dir,
    } = harness;
    let (done_tx, done_rx) = mpsc::channel();
    let shutdown = std::thread::spawn(move || {
        drop(shortcuts);
        let _ = done_tx.send(());
    });
    done_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("shutdown blocked on a portal request");
    shutdown.join().unwrap();
    assert!(call.join().unwrap().is_err());
}

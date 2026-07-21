use super::*;

pub(super) fn open_dbus_menu(
    conn: &Connection,
    session_id: u64,
    service: &str,
    item_path: &str,
) -> zbus::Result<Option<DbusMenuSession>> {
    let item = uncached_proxy(conn, service, item_path, ITEM_IFACE)?;
    let menu_path: String = item
        .get_property("Menu")
        .unwrap_or_else(|_| "/".to_string());
    if menu_path == "/" {
        return Ok(None);
    }

    notify_menu_about_to_show(conn, service, &menu_path, 0);
    let view = fetch_menu_level(conn, service, &menu_path, 0, false)?;
    if view.entries.is_empty() {
        return Ok(None);
    }
    Ok(Some(DbusMenuSession {
        id: session_id,
        service: service.to_string(),
        menu_path,
        parents: Vec::new(),
        last_view: view,
    }))
}

pub(super) fn handle_menu_action(
    conn: &Connection,
    action: MenuAction,
    evt_tx: &Sender<SystrayEvt>,
    session: &mut Option<DbusMenuSession>,
) {
    let Some(current) = session.as_mut() else {
        return;
    };
    match action {
        MenuAction::Activate(id) => {
            let session_id = current.id;
            if let Err(error) = send_menu_click(conn, &current.service, &current.menu_path, id) {
                log::warn!("status notifier: menu activation failed: {error}");
            }
            *session = None;
            send_menu_changed(evt_tx, session_id, None);
        }
        MenuAction::OpenSubmenu(id) => {
            notify_menu_about_to_show(conn, &current.service, &current.menu_path, id);
            match fetch_menu_level(conn, &current.service, &current.menu_path, id, true) {
                // A submenu level always contains the synthetic Back entry.
                // Do not navigate into a stale submenu that has no real items.
                Ok(view) if view.entries.len() > 1 => {
                    current.parents.push(id);
                    current.last_view = view.clone();
                    send_menu_changed(evt_tx, current.id, Some(view));
                }
                Ok(_) => {}
                Err(error) => log::warn!("status notifier: failed to open submenu: {error}"),
            }
        }
        MenuAction::Back => {
            current.parents.pop();
            let parent_id = current.parent_id();
            notify_menu_about_to_show(conn, &current.service, &current.menu_path, parent_id);
            match fetch_menu_level(
                conn,
                &current.service,
                &current.menu_path,
                parent_id,
                parent_id != 0,
            ) {
                Ok(view) => {
                    current.last_view = view.clone();
                    send_menu_changed(evt_tx, current.id, Some(view));
                }
                Err(error) => log::warn!("status notifier: failed to return to menu: {error}"),
            }
        }
    }
}

pub(super) fn refresh_menu_session(
    conn: &Connection,
    evt_tx: &Sender<SystrayEvt>,
    session: &mut Option<DbusMenuSession>,
) {
    let Some(current) = session.as_mut() else {
        return;
    };
    let parent_id = current.parent_id();
    match fetch_menu_level(
        conn,
        &current.service,
        &current.menu_path,
        parent_id,
        parent_id != 0,
    ) {
        Ok(view) if view != current.last_view => {
            current.last_view = view.clone();
            send_menu_changed(evt_tx, current.id, Some(view));
        }
        Ok(_) => {}
        Err(error) => {
            let session_id = current.id;
            log::debug!("status notifier: closing unavailable menu: {error}");
            *session = None;
            send_menu_changed(evt_tx, session_id, None);
        }
    }
}

pub(super) fn send_menu_changed(
    evt_tx: &Sender<SystrayEvt>,
    session_id: u64,
    view: Option<MenuView>,
) {
    let _ = evt_tx.send(SystrayEvt::MenuChanged { session_id, view });
}

fn notify_menu_about_to_show(conn: &Connection, service: &str, menu_path: &str, id: i32) {
    let Ok(proxy) = Proxy::new(conn, service, menu_path, DBUSMENU_IFACE) else {
        return;
    };
    // Some implementations return the specified boolean while others return
    // an empty body. Keep compatibility without invoking this stateful method
    // twice by deliberately leaving the single reply body uninterpreted.
    let _ = proxy.call_method("AboutToShow", &(id,));
}

fn fetch_menu_level(
    conn: &Connection,
    service: &str,
    menu_path: &str,
    parent_id: i32,
    include_back: bool,
) -> zbus::Result<MenuView> {
    let proxy = Proxy::new(conn, service, menu_path, DBUSMENU_IFACE)?;
    let root = match proxy
        .call::<_, _, (u32, OwnedValue)>("GetLayout", &(parent_id, 1i32, Vec::<String>::new()))
    {
        Ok((_, layout)) => layout,
        Err(_) => {
            let (layout,): (OwnedValue,) =
                proxy.call("GetLayout", &(parent_id, 1i32, Vec::<String>::new()))?;
            layout
        }
    };

    let (_, _, children): (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>) =
        root.try_into().map_err(zbus::Error::Variant)?;
    let mut entries = Vec::with_capacity(children.len() + usize::from(include_back));
    if include_back {
        entries.push(MenuEntry {
            label: "‹ Back".to_string(),
            width: 72,
            enabled: true,
            separator: false,
            toggle: MenuToggle::None,
            action: MenuAction::Back,
        });
    }
    for child in children {
        if let Some(entry) = parse_menu_entry(child)? {
            entries.push(entry);
        }
    }
    Ok(MenuView { entries })
}

fn parse_menu_entry(value: OwnedValue) -> zbus::Result<Option<MenuEntry>> {
    let (id, props, children): (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>) =
        value.try_into().map_err(zbus::Error::Variant)?;
    Ok(menu_entry_from_properties(id, &props, !children.is_empty()))
}

pub(super) fn menu_entry_from_properties(
    id: i32,
    props: &HashMap<String, OwnedValue>,
    has_layout_children: bool,
) -> Option<MenuEntry> {
    if !menu_prop_bool(props, "visible").unwrap_or(true) {
        return None;
    }

    let separator = menu_prop_string(props, "type").is_some_and(|kind| kind == "separator");
    let raw_label = menu_prop_string(props, "label").unwrap_or_default();
    let label = strip_menu_mnemonics(&raw_label).trim().to_string();
    if !separator && label.is_empty() {
        return None;
    }
    let enabled = !separator && menu_prop_bool(props, "enabled").unwrap_or(true);
    let has_submenu = has_layout_children
        || menu_prop_string(props, "children-display").is_some_and(|value| value == "submenu");
    let toggle_state = menu_prop_i32(props, "toggle-state").unwrap_or(0) == 1;
    let toggle = match menu_prop_string(props, "toggle-type").as_deref() {
        Some("checkmark") => MenuToggle::Check(toggle_state),
        Some("radio") => MenuToggle::Radio(toggle_state),
        _ => MenuToggle::None,
    };
    let indicator_width = if toggle == MenuToggle::None { 0 } else { 16 };
    let submenu_width = if has_submenu { 16 } else { 0 };
    let width = if separator {
        24
    } else {
        (label.chars().count() as i32 * 8 + 20 + indicator_width + submenu_width).max(24)
    };
    Some(MenuEntry {
        label,
        width,
        enabled,
        separator,
        toggle,
        action: if has_submenu {
            MenuAction::OpenSubmenu(id)
        } else {
            MenuAction::Activate(id)
        },
    })
}

pub(super) fn strip_menu_mnemonics(label: &str) -> String {
    let mut chars = label.chars().peekable();
    let mut stripped = String::with_capacity(label.len());
    while let Some(ch) = chars.next() {
        if ch != '_' {
            stripped.push(ch);
            continue;
        }
        if chars.peek() == Some(&'_') {
            chars.next();
            stripped.push('_');
        }
    }
    stripped
}

fn menu_prop_string(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let value = props.get(key)?;
    String::try_from(value.clone())
        .ok()
        .or_else(|| <&str>::try_from(value).ok().map(str::to_string))
}

fn menu_prop_bool(props: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    props.get(key).and_then(|value| bool::try_from(value).ok())
}

fn menu_prop_i32(props: &HashMap<String, OwnedValue>, key: &str) -> Option<i32> {
    props.get(key).and_then(|value| i32::try_from(value).ok())
}

fn send_menu_click(conn: &Connection, service: &str, menu_path: &str, id: i32) -> zbus::Result<()> {
    let proxy = Proxy::new(conn, service, menu_path, DBUSMENU_IFACE)?;
    let _: () = proxy.call("Event", &(id, "clicked", Value::new(""), 0u32))?;
    Ok(())
}

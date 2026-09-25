use crate::ipc_types::{Response, TagCommand, TagInfo};
use crate::tags::{name_tag, reset_name_tag};
use crate::wm::Wm;

pub fn handle_tag_command(wm: &mut Wm, cmd: TagCommand) -> Response {
    match cmd {
        TagCommand::List => return list_tags(wm),
        TagCommand::Name { name } => name_tag(&mut wm.ctx(), &name),
        TagCommand::Reset => reset_name_tag(&mut wm.ctx()),
    }
    Response::ok()
}

/// Describe every tag of the selected monitor: configured name and icon,
/// the label the bar currently shows, and whether the tag is occupied or
/// selected.
fn list_tags(wm: &Wm) -> Response {
    let core = &wm.core;
    let monitor = core.model.expect_selected_monitor();
    let show_icons = core.config.tags.show_icons;
    let occupied = monitor.occupied_tags(&core.model.clients);
    let selected = monitor.visible_tags();

    let tags = monitor
        .tags
        .iter()
        .enumerate()
        .map(|(index, tag)| TagInfo {
            index: index as u32,
            name: (!tag.name.is_empty()).then(|| tag.name.clone()),
            icon: (!tag.icon.is_empty()).then(|| tag.icon.clone()),
            label: tag.display_label(show_icons).to_string(),
            occupied: occupied.contains(index + 1),
            selected: selected.contains(index + 1),
        })
        .collect();

    Response::TagList(tags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, wayland::WaylandBackend};
    use crate::config::config_toml::TagsConfig;
    use crate::config::resolve_config;
    use crate::types::{Client, Monitor, Rect, TagMask, WindowId};

    fn wm(show_icons: bool) -> Wm {
        let mut user: crate::config::config_toml::UserConfig = toml::from_str("").unwrap();
        user.tags = TagsConfig {
            names: vec!["web".into(), "mail".into(), "code".into()],
            icons: vec!["W".into()],
            show_icons,
        };
        let config = resolve_config(user, crate::backend::BackendKind::Wayland).unwrap();
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 800, 600),
            ..Monitor::default()
        });
        wm.core.apply_config(config);
        wm.core
            .model
            .expect_selected_monitor_mut()
            .set_selected_tags(TagMask::single(2).unwrap());
        wm
    }

    fn list(wm: &mut Wm) -> Vec<TagInfo> {
        match handle_tag_command(wm, TagCommand::List) {
            Response::TagList(tags) => tags,
            other => panic!("expected TagList, got {other:?}"),
        }
    }

    #[test]
    fn list_reports_names_icons_and_the_active_label() {
        let mut wm = wm(false);
        let monitor_id = wm.core.model.selected_monitor_id();
        wm.core.model.insert_client(Client {
            win: WindowId(1),
            monitor_id,
            geo: Rect::new(0, 0, 10, 10),
            tags: TagMask::single(1).unwrap(),
            ..Client::default()
        });
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .clients
            .push(WindowId(1));

        let tags = list(&mut wm);
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0].name.as_deref(), Some("web"));
        assert_eq!(tags[0].icon.as_deref(), Some("W"));
        // Icon mode off: the name is shown even though an icon exists.
        assert_eq!(tags[0].label, "web");
        assert!(tags[0].occupied);
        assert!(!tags[0].selected);
        // No icon configured: `icon` is None and the name is the label.
        assert_eq!(tags[1].icon, None);
        assert_eq!(tags[1].label, "mail");
        assert!(tags[1].selected);
        assert!(!tags[1].occupied);

        // Icon mode on: the icon wins where one exists, names elsewhere.
        wm.core.config.tags.show_icons = true;
        let tags = list(&mut wm);
        assert_eq!(tags[0].label, "W");
        assert_eq!(tags[1].label, "mail");
    }
}

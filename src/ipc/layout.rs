//! Window-layout queries: list the cycle entries and report the selected
//! monitor's full layout state.

use crate::contexts::WmCtx;
use crate::ipc_types::{LayoutInfo, LayoutStatusInfo, Response};
use crate::layouts::{LayoutCommand, PresentationMode};

fn layout_info(command: LayoutCommand, is_active: bool) -> LayoutInfo {
    LayoutInfo {
        name: command.name().to_string(),
        label: command.label().to_string(),
        symbol: command.symbol().to_string(),
        is_active,
    }
}

pub fn list_layouts(ctx: &WmCtx<'_>) -> Response {
    let active = ctx
        .state()
        .model
        .expect_selected_monitor()
        .current_layout_command();
    Response::LayoutList(
        LayoutCommand::all()
            .iter()
            .map(|&command| layout_info(command, command == active))
            .collect(),
    )
}

pub fn layout_status(ctx: &WmCtx<'_>) -> Response {
    let monitor = ctx.model().expect_selected_monitor();
    let presentation = match monitor.current_layout() {
        PresentationMode::Tiled => "tiled",
        PresentationMode::Floating => "floating",
        PresentationMode::Maximized => "maximized",
    };
    Response::LayoutStatus(LayoutStatusInfo {
        monitor_id: monitor.id().get(),
        presentation: presentation.to_string(),
        layout: layout_info(monitor.current_layout_command(), true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MonitorBuilder;
    use crate::test_support::TestWm as Wm;
    use crate::types::{Client, ClientMode, Rect, TagMask, WindowId};

    fn wm_with_monitor() -> Wm {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        let tags = TagMask::single(1).unwrap();
        let monitor_id = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(Rect::new(0, 0, 1200, 800))
                .tag_count(1)
                .selected_tags(tags)
                .build(),
        );
        wm.core.state.model.monitors.set_selected(monitor_id);
        let mut client = Client {
            win: WindowId(1),
            tags,
            ..Client::default()
        };
        client.set_mode_for_test(ClientMode::tiled());
        wm.core.state.model.readopt_client(monitor_id, client, true);
        wm
    }

    fn active_names(response: Response) -> Vec<String> {
        let Response::LayoutList(layouts) = response else {
            panic!("expected a layout list");
        };
        layouts
            .iter()
            .filter(|layout| layout.is_active)
            .map(|layout| layout.name.clone())
            .collect()
    }

    #[test]
    fn listing_marks_exactly_one_active_entry() {
        let mut wm = wm_with_monitor();

        let names: Vec<String> = match list_layouts(&wm.test_ctx()) {
            Response::LayoutList(layouts) => layouts.iter().map(|l| l.name.clone()).collect(),
            other => panic!("expected a layout list, got {other:?}"),
        };
        assert_eq!(
            names,
            ["tile", "grid", "floating", "maximized", "bottom-stack"]
        );
        assert_eq!(active_names(list_layouts(&wm.test_ctx())), ["tile"]);
    }

    #[test]
    fn status_reports_the_active_slot_and_presentation() {
        let mut wm = wm_with_monitor();
        crate::layouts::set_layout(&mut wm.test_ctx(), LayoutCommand::Grid);

        let Response::LayoutStatus(status) = layout_status(&wm.test_ctx()) else {
            panic!("expected a layout status");
        };
        assert_eq!(status.layout.name, "grid");
        assert_eq!(status.layout.symbol, "#");
        assert_eq!(status.presentation, "tiled");
        assert!(status.layout.is_active);
    }

    #[test]
    fn status_reports_the_lens_entry_while_lensed() {
        let mut wm = wm_with_monitor();
        crate::layouts::set_layout(&mut wm.test_ctx(), LayoutCommand::Grid);
        crate::layouts::set_layout(&mut wm.test_ctx(), LayoutCommand::Maximized);

        // The lens is the active entry; the grid slot stays underneath.
        assert_eq!(active_names(list_layouts(&wm.test_ctx())), ["maximized"]);
        let Response::LayoutStatus(status) = layout_status(&wm.test_ctx()) else {
            panic!("expected a layout status");
        };
        assert_eq!(status.layout.name, "maximized");
        assert_eq!(status.presentation, "maximized");
    }
}

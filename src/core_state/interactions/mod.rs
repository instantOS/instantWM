//! Backend-neutral pointer and touch interaction state.
//!
//! Backends feed these state machines normalized input. Window-system grabs,
//! event loops, cursor updates, and other side effects remain in crate::mouse
//! and crate::backend.

use super::*;

mod gestures;
mod hover;
mod window;

pub use gestures::*;
pub use hover::*;
pub use window::*;

#[cfg(test)]
mod tests;

/// The single compositor-owned input sequence currently in progress.
///
/// Making capture mutually exclusive in the type system removes the former
/// priority ordering between several independent `Option` fields. Backends
/// can now decide whether to forward an event from this one source of truth.
#[derive(Debug, Clone)]
pub enum CapturedInteraction {
    Window(WindowDragState),
    Tag(TagDragState),
    SidebarVolume(SidebarVolumeDrag),
    BottomBar(BottomBarDrag),
    OverviewCard(OverviewCardDrag),
}

impl CapturedInteraction {
    pub fn button(&self) -> MouseButton {
        match self {
            Self::Window(state) => state.interaction().button(),
            Self::Tag(state) => state.button,
            Self::SidebarVolume(state) => state.button(),
            Self::BottomBar(state) => state.button(),
            Self::OverviewCard(state) => state.button(),
        }
    }

    pub fn source(&self) -> InteractionSource {
        match self {
            Self::Window(state) => state.interaction().source(),
            Self::Tag(state) => state.source,
            Self::SidebarVolume(state) => state.source(),
            Self::BottomBar(state) => state.source(),
            Self::OverviewCard(state) => state.source(),
        }
    }
}

/// Consolidated state for mouse/touch interactions.
#[derive(Debug, Clone, Default)]
pub struct DragState {
    capture: Option<CapturedInteraction>,
    pub hover_offer: HoverOffer,
}

impl DragState {
    pub fn capture(&self) -> Option<&CapturedInteraction> {
        self.capture.as_ref()
    }

    pub fn has_capture(&self) -> bool {
        self.capture.is_some()
    }

    pub fn active_interaction(&self) -> Option<&DragInteraction> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::Window(WindowDragState::Active(drag))) => Some(drag),
            _ => None,
        }
    }

    pub fn armed_interaction(&self) -> Option<&DragInteraction> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::Window(WindowDragState::Armed(drag))) => Some(drag),
            _ => None,
        }
    }

    pub fn reordering_interaction(&self) -> Option<(&DragInteraction, &TitleReorderDrag)> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::Window(WindowDragState::Reordering(drag, reorder))) => {
                Some((drag, reorder))
            }
            _ => None,
        }
    }

    /// Button whose complete press/motion/release sequence is WM-owned.
    pub fn captured_button(&self) -> Option<MouseButton> {
        self.capture.as_ref().map(CapturedInteraction::button)
    }

    pub fn captured_source(&self) -> Option<InteractionSource> {
        self.capture.as_ref().map(CapturedInteraction::source)
    }

    pub fn tag_drag(&self) -> Option<&TagDragState> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::Tag(drag)) => Some(drag),
            _ => None,
        }
    }

    pub fn tag_drag_mut(&mut self) -> Option<&mut TagDragState> {
        match self.capture.as_mut() {
            Some(CapturedInteraction::Tag(drag)) => Some(drag),
            _ => None,
        }
    }

    pub fn begin_tag_drag(&mut self, drag: TagDragState) -> Result<(), InteractionAlreadyActive> {
        self.begin_capture(CapturedInteraction::Tag(drag))
    }

    pub fn finish_tag_drag(&mut self, button: MouseButton) -> Option<TagDragState> {
        if !matches!(
            self.capture.as_ref(),
            Some(CapturedInteraction::Tag(drag)) if drag.button == button
        ) {
            return None;
        }
        match self.capture.take() {
            Some(CapturedInteraction::Tag(drag)) => Some(drag),
            _ => unreachable!(),
        }
    }

    pub fn begin_overview_card(
        &mut self,
        drag: OverviewCardDrag,
    ) -> Result<(), InteractionAlreadyActive> {
        self.begin_capture(CapturedInteraction::OverviewCard(drag))
    }

    pub fn update_overview_card(&mut self, root: Point) -> Option<bool> {
        match self.capture.as_mut() {
            Some(CapturedInteraction::OverviewCard(drag)) => drag.update(root),
            _ => None,
        }
    }

    pub fn finish_overview_card(&mut self, button: MouseButton) -> Option<OverviewCardAction> {
        if !matches!(
            self.capture.as_ref(),
            Some(CapturedInteraction::OverviewCard(drag)) if drag.button() == button
        ) {
            return None;
        }
        match self.capture.take() {
            Some(CapturedInteraction::OverviewCard(drag)) => Some(drag.action()),
            _ => unreachable!(),
        }
    }

    pub fn cancel_overview_card(&mut self) -> bool {
        if !matches!(self.capture, Some(CapturedInteraction::OverviewCard(_))) {
            return false;
        }
        self.capture = None;
        true
    }

    pub fn sidebar_volume_active(&self) -> bool {
        matches!(self.capture, Some(CapturedInteraction::SidebarVolume(_)))
    }

    pub fn sidebar_volume_button(&self) -> Option<MouseButton> {
        match self.capture {
            Some(CapturedInteraction::SidebarVolume(drag)) => Some(drag.button()),
            _ => None,
        }
    }

    pub fn sidebar_volume_monitor(&self) -> Option<MonitorId> {
        match self.capture {
            Some(CapturedInteraction::SidebarVolume(drag)) => Some(drag.monitor_id()),
            _ => None,
        }
    }

    pub fn begin_sidebar_volume(
        &mut self,
        drag: SidebarVolumeDrag,
    ) -> Result<(), InteractionAlreadyActive> {
        self.begin_capture(CapturedInteraction::SidebarVolume(drag))
    }

    pub fn update_sidebar_volume(&mut self, root_y: i32) -> Option<i32> {
        match self.capture.as_mut() {
            Some(CapturedInteraction::SidebarVolume(drag)) => Some(drag.update(root_y)),
            _ => None,
        }
    }

    pub fn finish_sidebar_volume(&mut self, button: MouseButton) -> bool {
        if self.sidebar_volume_button() != Some(button) {
            return false;
        }
        self.capture = None;
        true
    }

    pub fn cancel_sidebar_volume(&mut self) -> bool {
        if !self.sidebar_volume_active() {
            return false;
        }
        self.capture = None;
        true
    }

    pub fn bottom_bar_gesture_active(&self) -> bool {
        matches!(self.capture, Some(CapturedInteraction::BottomBar(_)))
    }

    pub fn bottom_bar_button(&self) -> Option<MouseButton> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::BottomBar(drag)) => Some(drag.button()),
            _ => None,
        }
    }

    pub fn bottom_bar_monitor(&self) -> Option<MonitorId> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::BottomBar(drag)) => Some(drag.monitor_id()),
            _ => None,
        }
    }

    pub fn bottom_bar_drag(&self) -> Option<&BottomBarDrag> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::BottomBar(drag)) => Some(drag),
            _ => None,
        }
    }

    pub fn begin_bottom_bar(
        &mut self,
        drag: BottomBarDrag,
    ) -> Result<(), InteractionAlreadyActive> {
        self.begin_capture(CapturedInteraction::BottomBar(drag))
    }

    pub fn update_bottom_bar(&mut self, root: Point) -> Option<SwipeDirection> {
        match self.capture.as_mut() {
            Some(CapturedInteraction::BottomBar(drag)) => drag.update(root),
            _ => None,
        }
    }

    pub fn finish_bottom_bar(&mut self, button: MouseButton) -> bool {
        if self.bottom_bar_button() != Some(button) {
            return false;
        }
        self.capture = None;
        true
    }

    pub fn cancel_bottom_bar(&mut self) -> bool {
        if !self.bottom_bar_gesture_active() {
            return false;
        }
        self.capture = None;
        true
    }

    pub fn begin_move(
        &mut self,
        win: WindowId,
        button: MouseButton,
        source: InteractionSource,
        start: Point,
        geo: Rect,
    ) -> Result<(), InteractionAlreadyActive> {
        self.begin_active(DragInteraction::immediate(
            win,
            button,
            source,
            DragOperation::Move,
            start,
            geo,
        ))
    }

    pub fn begin_resize(
        &mut self,
        win: WindowId,
        button: MouseButton,
        source: InteractionSource,
        dir: ResizeDirection,
        start: Point,
        geo: Rect,
    ) -> Result<(), InteractionAlreadyActive> {
        self.begin_resize_with_policy(ActiveResizeParams {
            win,
            button,
            source,
            direction: dir,
            start,
            geometry: geo,
            policy: ResizePolicy::Free,
        })
    }

    pub fn begin_resize_with_policy(
        &mut self,
        params: ActiveResizeParams,
    ) -> Result<(), InteractionAlreadyActive> {
        let mut drag = DragInteraction::immediate(
            params.win,
            params.button,
            params.source,
            DragOperation::Resize(params.direction),
            params.start,
            params.geometry,
        );
        drag.set_resize_policy(params.policy);
        self.begin_active(drag)
    }

    pub fn begin_tree_resize(
        &mut self,
        params: TreeResizeParams,
    ) -> Result<(), InteractionAlreadyActive> {
        self.begin_active(DragInteraction::immediate(
            params.win,
            params.button,
            params.source,
            DragOperation::TreeResize {
                direction: params.direction,
                origin: params.origin,
            },
            params.start,
            params.geometry,
        ))
    }

    fn begin_active(&mut self, drag: DragInteraction) -> Result<(), InteractionAlreadyActive> {
        self.begin_capture(CapturedInteraction::Window(WindowDragState::Active(drag)))
    }

    pub fn arm_title_drag(
        &mut self,
        params: ArmedDragParams,
    ) -> Result<(), InteractionAlreadyActive> {
        self.begin_capture(CapturedInteraction::Window(WindowDragState::Armed(
            DragInteraction::armed(params),
        )))
    }

    pub fn activate_armed(
        &mut self,
        drag_type: ArmedDragType,
        start: Point,
        geo: Rect,
    ) -> Result<(), DragNotArmed> {
        let mut drag = match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Armed(drag))) => drag,
            other => {
                self.capture = other;
                return Err(DragNotArmed);
            }
        };
        drag.activate_as(drag_type, start, geo);
        self.capture = Some(CapturedInteraction::Window(WindowDragState::Active(drag)));
        Ok(())
    }

    /// Promote an armed bar-title press to a live title-strip reorder.
    pub fn begin_title_reorder(&mut self, reorder: TitleReorderDrag) -> Result<(), DragNotArmed> {
        match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Armed(drag))) => {
                self.capture = Some(CapturedInteraction::Window(WindowDragState::Reordering(
                    drag, reorder,
                )));
                Ok(())
            }
            other => {
                self.capture = other;
                Err(DragNotArmed)
            }
        }
    }

    /// Convert a live title-strip reorder into an ordinary active move drag.
    pub fn activate_reordering_as_move(
        &mut self,
        start: Point,
        geo: Rect,
    ) -> Result<(), DragNotArmed> {
        let mut drag = match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Reordering(drag, _))) => drag,
            other => {
                self.capture = other;
                return Err(DragNotArmed);
            }
        };
        drag.activate_as(ArmedDragType::Move, start, geo);
        self.capture = Some(CapturedInteraction::Window(WindowDragState::Active(drag)));
        Ok(())
    }

    pub fn finish_reordering(&mut self) -> Option<DragInteraction> {
        match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Reordering(drag, _))) => Some(drag),
            other => {
                self.capture = other;
                None
            }
        }
    }

    pub fn record_interactive_motion(&mut self, point: Point) {
        if let Some(CapturedInteraction::Window(
            WindowDragState::Armed(drag)
            | WindowDragState::Reordering(drag, _)
            | WindowDragState::Active(drag),
        )) = self.capture.as_mut()
        {
            drag.record_motion(point)
        }
    }

    pub fn finish_active(&mut self, button: MouseButton) -> Option<DragInteraction> {
        if !self
            .active_interaction()
            .is_some_and(|drag| drag.button() == button)
        {
            return None;
        }
        match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Active(drag))) => Some(drag),
            _ => unreachable!(),
        }
    }

    pub fn finish_armed(&mut self) -> Option<DragInteraction> {
        match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Armed(drag))) => Some(drag),
            other => {
                self.capture = other;
                None
            }
        }
    }

    pub fn cancel_interactive(&mut self) -> Option<DragInteraction> {
        match self.capture.take() {
            Some(CapturedInteraction::Window(
                WindowDragState::Armed(drag)
                | WindowDragState::Reordering(drag, _)
                | WindowDragState::Active(drag),
            )) => Some(drag),
            other => {
                self.capture = other;
                None
            }
        }
    }

    pub fn cancel_capture(&mut self) -> Option<CapturedInteraction> {
        self.capture.take()
    }

    fn begin_capture(
        &mut self,
        capture: CapturedInteraction,
    ) -> Result<(), InteractionAlreadyActive> {
        if self.capture.is_some() {
            return Err(InteractionAlreadyActive);
        }
        self.capture = Some(capture);
        Ok(())
    }

    #[inline]
    pub fn set_hover_offer(&mut self, offer: HoverOffer) {
        self.hover_offer = offer;
    }

    /// Clears an active hover offer. Returns `true` if the state changed.
    pub fn clear_hover_offer(&mut self) -> bool {
        if !self.hover_offer.is_active() {
            return false;
        }
        self.hover_offer = HoverOffer::None;
        true
    }
}

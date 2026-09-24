//! Backend-neutral pointer and touch interaction state.
//!
//! Backends feed these state machines normalized input. Native event loops and
//! input-capture mechanisms remain backend concerns; the cursor and pointer
//! routing they must present are derived from this authoritative state.

use super::*;

mod gestures;
mod hover;
mod window;

pub use gestures::*;
pub use hover::*;
pub use window::*;

#[cfg(test)]
mod tests;

/// Backend-neutral presentation required by the current interaction state.
///
/// This is a level-triggered description, not a request to perform a native
/// operation. Backends reconcile their current cursor and input ownership with
/// this value, making redundant synchronization safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionProjection {
    pub cursor: AltCursor,
    pub pointer_delivery: PointerDelivery,
    /// Window undergoing a direct interactive geometry resize. Semantic tree
    /// weight resizing is intentionally not a client resize lifecycle.
    pub active_resize_window: Option<WindowId>,
}

impl Default for InteractionProjection {
    fn default() -> Self {
        Self {
            cursor: AltCursor::Default,
            pointer_delivery: PointerDelivery::Default,
            active_resize_window: None,
        }
    }
}

/// Who must receive the pointer stream represented by an interaction.
///
/// Backends are free to satisfy this guarantee differently. In particular,
/// X11 uses a native pointer grab for [`Self::DeliverHoverCommitToWm`], while
/// Wayland's compositor input path already owns the stream.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PointerDelivery {
    #[default]
    Default,
    DeliverHoverCommitToWm,
}

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

/// A capture variant addressed by its state type, so one set of generic
/// accessors serves every kind of captured interaction.
pub trait CaptureKind: Sized + Into<CapturedInteraction> {
    fn get(capture: &CapturedInteraction) -> Option<&Self>;
    fn get_mut(capture: &mut CapturedInteraction) -> Option<&mut Self>;
    fn take(capture: CapturedInteraction) -> Option<Self>;
}

macro_rules! capture_kinds {
    ($($variant:ident($state:ty)),* $(,)?) => {$(
        impl From<$state> for CapturedInteraction {
            fn from(state: $state) -> Self {
                Self::$variant(state)
            }
        }

        impl CaptureKind for $state {
            fn get(capture: &CapturedInteraction) -> Option<&Self> {
                match capture {
                    CapturedInteraction::$variant(state) => Some(state),
                    _ => None,
                }
            }

            fn get_mut(capture: &mut CapturedInteraction) -> Option<&mut Self> {
                match capture {
                    CapturedInteraction::$variant(state) => Some(state),
                    _ => None,
                }
            }

            fn take(capture: CapturedInteraction) -> Option<Self> {
                match capture {
                    CapturedInteraction::$variant(state) => Some(state),
                    _ => None,
                }
            }
        }
    )*};
}

capture_kinds!(
    Window(WindowDragState),
    Tag(TagDragState),
    SidebarVolume(SidebarVolumeDrag),
    BottomBar(BottomBarDrag),
    OverviewCard(OverviewCardDrag),
);

impl CapturedInteraction {
    pub fn button(&self) -> MouseButton {
        match self {
            Self::Window(state) => state.button(),
            Self::Tag(state) => state.button,
            Self::SidebarVolume(state) => state.button,
            Self::BottomBar(state) => state.button,
            Self::OverviewCard(state) => state.button,
        }
    }

    pub fn source(&self) -> InteractionSource {
        match self {
            Self::Window(state) => state.source(),
            Self::Tag(state) => state.source,
            Self::SidebarVolume(state) => state.source,
            Self::BottomBar(state) => state.source,
            Self::OverviewCard(state) => state.source,
        }
    }

    /// Whether this interaction owns the built-in bar's hover presentation.
    ///
    /// Input adapters must not run ordinary hover updates while this is true:
    /// doing so races the gesture's highlight state on every motion sample.
    pub fn owns_bar_hover(&self) -> bool {
        matches!(
            self,
            Self::Tag(_) | Self::Window(WindowDragState::Reordering(..))
        )
    }

    fn cursor(&self) -> AltCursor {
        match self {
            Self::Window(WindowDragState::Armed(_)) => AltCursor::Default,
            Self::Window(WindowDragState::Reordering(..)) => AltCursor::HorizontalAdjust,
            Self::Window(WindowDragState::Active(drag)) => drag.operation().cursor(),
            Self::Tag(drag) if drag.dragging => AltCursor::Move,
            Self::Tag(_) => AltCursor::Default,
            Self::SidebarVolume(_) => AltCursor::VerticalAdjust,
            Self::BottomBar(drag) => match drag.latched_direction() {
                Some(SwipeDirection::Up) => AltCursor::VerticalAdjust,
                Some(SwipeDirection::Left | SwipeDirection::Right) => AltCursor::HorizontalAdjust,
                None => AltCursor::Move,
            },
            Self::OverviewCard(drag) if drag.close_armed() => AltCursor::Close,
            Self::OverviewCard(_) => AltCursor::Move,
        }
    }
}

/// Authoritative state for compositor-owned pointer and touch interactions.
#[derive(Debug, Clone, Default)]
pub struct PointerInteractionState {
    capture: Option<CapturedInteraction>,
    hover_offer: HoverOffer,
}

impl PointerInteractionState {
    /// Derive the complete native presentation from authoritative interaction
    /// state. A captured sequence always takes precedence over a passive hover
    /// offer, so cursor policy cannot depend on imperative call ordering.
    pub fn projection(&self) -> InteractionProjection {
        if let Some(capture) = self.capture.as_ref() {
            return InteractionProjection {
                cursor: capture.cursor(),
                pointer_delivery: PointerDelivery::Default,
                active_resize_window: match capture {
                    CapturedInteraction::Window(WindowDragState::Active(drag))
                        if drag.operation().is_direct_resize() =>
                    {
                        Some(drag.win())
                    }
                    _ => None,
                },
            };
        }
        match self.hover_offer {
            HoverOffer::Resize { dir, .. } | HoverOffer::TreeResize { dir, .. } => {
                InteractionProjection {
                    cursor: AltCursor::Resize(dir),
                    pointer_delivery: PointerDelivery::DeliverHoverCommitToWm,
                    active_resize_window: None,
                }
            }
            HoverOffer::Sidebar(_) => InteractionProjection {
                cursor: AltCursor::VerticalAdjust,
                pointer_delivery: PointerDelivery::Default,
                active_resize_window: None,
            },
            HoverOffer::None => InteractionProjection::default(),
        }
    }

    pub fn hover_offer(&self) -> HoverOffer {
        self.hover_offer
    }

    pub fn capture(&self) -> Option<&CapturedInteraction> {
        self.capture.as_ref()
    }

    pub fn has_capture(&self) -> bool {
        self.capture.is_some()
    }

    pub fn owns_bar_hover(&self) -> bool {
        self.capture
            .as_ref()
            .is_some_and(CapturedInteraction::owns_bar_hover)
    }

    pub fn active_interaction(&self) -> Option<&ActiveWindowDrag> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::Window(WindowDragState::Active(drag))) => Some(drag),
            _ => None,
        }
    }

    pub fn armed_interaction(&self) -> Option<&ArmedWindowDrag> {
        match self.capture.as_ref() {
            Some(CapturedInteraction::Window(WindowDragState::Armed(drag))) => Some(drag),
            _ => None,
        }
    }

    pub fn reordering_interaction(&self) -> Option<(&ArmedWindowDrag, &TitleReorderDrag)> {
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

    pub fn captured<T: CaptureKind>(&self) -> Option<&T> {
        self.capture.as_ref().and_then(T::get)
    }

    pub fn captured_mut<T: CaptureKind>(&mut self) -> Option<&mut T> {
        self.capture.as_mut().and_then(T::get_mut)
    }

    /// End a `T` capture, but only on release of the button that started it.
    pub fn finish<T: CaptureKind>(&mut self, button: MouseButton) -> Option<T> {
        self.capture
            .take_if(|capture| capture.button() == button && T::get(capture).is_some())
            .and_then(T::take)
    }

    /// End a `T` capture regardless of its button.
    pub fn cancel<T: CaptureKind>(&mut self) -> Option<T> {
        self.capture
            .take_if(|capture| T::get(capture).is_some())
            .and_then(T::take)
    }

    pub fn begin_move(
        &mut self,
        win: WindowId,
        button: MouseButton,
        source: InteractionSource,
        start: Point,
        geo: Rect,
    ) -> Result<(), InteractionAlreadyActive> {
        self.begin_active(ActiveWindowDrag::immediate(
            win,
            button,
            source,
            ActiveWindowOperation::Move,
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
        self.begin_resize_with_policy(DirectResizeStart {
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
        params: DirectResizeStart,
    ) -> Result<(), InteractionAlreadyActive> {
        let drag = ActiveWindowDrag::immediate(
            params.win,
            params.button,
            params.source,
            ActiveWindowOperation::DirectResize {
                direction: params.direction,
                policy: params.policy,
            },
            params.start,
            params.geometry,
        );
        self.begin_active(drag)
    }

    pub fn begin_tree_resize(
        &mut self,
        params: TreeResizeStart,
    ) -> Result<(), InteractionAlreadyActive> {
        self.begin_active(ActiveWindowDrag::immediate(
            params.win,
            params.button,
            params.source,
            ActiveWindowOperation::TreeResize {
                direction: params.direction,
                origin: params.origin,
            },
            params.start,
            params.geometry,
        ))
    }

    fn begin_active(&mut self, drag: ActiveWindowDrag) -> Result<(), InteractionAlreadyActive> {
        self.begin(WindowDragState::Active(drag))
    }

    pub fn arm_title_drag(
        &mut self,
        params: ArmedDragStart,
    ) -> Result<(), InteractionAlreadyActive> {
        self.begin(WindowDragState::Armed(ArmedWindowDrag::new(params)))
    }

    pub(crate) fn activate_armed(
        &mut self,
        operation: ActiveWindowOperation,
        start: Point,
        geo: Rect,
    ) -> Result<(), DragNotArmed> {
        let drag = match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Armed(drag))) => drag,
            other => {
                self.capture = other;
                return Err(DragNotArmed);
            }
        };
        self.capture = Some(CapturedInteraction::Window(WindowDragState::Active(
            drag.activate(operation, start, geo),
        )));
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
        let drag = match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Reordering(drag, _))) => drag,
            other => {
                self.capture = other;
                return Err(DragNotArmed);
            }
        };
        self.capture = Some(CapturedInteraction::Window(WindowDragState::Active(
            drag.activate(ActiveWindowOperation::Move, start, geo),
        )));
        Ok(())
    }

    pub fn finish_reordering(&mut self) -> Option<ArmedWindowDrag> {
        match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Reordering(drag, _))) => Some(drag),
            other => {
                self.capture = other;
                None
            }
        }
    }

    pub fn record_interactive_motion(&mut self, point: Point) {
        if let Some(CapturedInteraction::Window(state)) = self.capture.as_mut() {
            match state {
                WindowDragState::Armed(drag) | WindowDragState::Reordering(drag, _) => {
                    drag.record_motion(point)
                }
                WindowDragState::Active(drag) => drag.record_motion(point),
            }
        }
    }

    pub fn finish_active(&mut self, button: MouseButton) -> Option<ActiveWindowDrag> {
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

    pub fn finish_armed(&mut self) -> Option<ArmedWindowDrag> {
        match self.capture.take() {
            Some(CapturedInteraction::Window(WindowDragState::Armed(drag))) => Some(drag),
            other => {
                self.capture = other;
                None
            }
        }
    }

    pub fn cancel_capture(&mut self) -> Option<CapturedInteraction> {
        self.capture.take()
    }

    pub fn begin(
        &mut self,
        capture: impl Into<CapturedInteraction>,
    ) -> Result<(), InteractionAlreadyActive> {
        if self.capture.is_some() {
            return Err(InteractionAlreadyActive);
        }
        // A passive offer and an owned input sequence are mutually exclusive.
        // Dropping the offer here prevents stale hover intent from resurfacing
        // when the capture later ends.
        self.hover_offer = HoverOffer::None;
        self.capture = Some(capture.into());
        Ok(())
    }

    /// Replace the passive offer and report whether authoritative state
    /// changed. Native projection is deliberately handled by `WmCtx` after
    /// the model transition.
    #[inline]
    pub fn set_hover_offer(&mut self, offer: HoverOffer) -> bool {
        if self.capture.is_some() {
            return false;
        }
        if self.hover_offer == offer {
            return false;
        }
        self.hover_offer = offer;
        true
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

use super::*;

/// The pointer-owned interaction currently being offered before a click commits it.
///
/// This is the source of truth for hover offers; cursor and pointer-routing
/// presentation are derived from it, not independently mutated side effects.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum HoverOffer {
    #[default]
    None,
    /// Cursor is in the resize border zone of a floating window.
    Resize { win: WindowId, dir: ResizeDirection },
    /// Cursor is in an inner gap on an adjustable tiled-tree seam.
    TreeResize { win: WindowId, dir: ResizeDirection },
    /// Cursor is on the sidebar drag edge.
    Sidebar(SidebarTarget),
}

impl HoverOffer {
    /// Whether any hover interaction is offered (not [`HoverOffer::None`]).
    #[inline]
    pub fn is_active(self) -> bool {
        !matches!(self, HoverOffer::None)
    }

    #[inline]
    pub fn is_sidebar(self) -> bool {
        matches!(self, HoverOffer::Sidebar(_))
    }

    /// The resize target and direction when this is a border-resize offer.
    #[inline]
    pub fn resize_target(self) -> Option<(WindowId, ResizeDirection)> {
        match self {
            HoverOffer::Resize { win, dir } => Some((win, dir)),
            _ => None,
        }
    }

    /// The tiled-tree seam targeted by an inner-gap offer.
    #[inline]
    pub fn tree_resize_target(self) -> Option<(WindowId, ResizeDirection)> {
        match self {
            HoverOffer::TreeResize { win, dir } => Some((win, dir)),
            _ => None,
        }
    }
}

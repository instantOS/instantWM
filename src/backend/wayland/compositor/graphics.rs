//! Shared ownership of native graphics, borrowed at dispatch/frame boundaries.

use std::cell::RefCell;
use std::rc::Rc;

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::WinitGraphicsBackend;

/// Nested Smithay graphics owns its renderer and lends it together with a
/// framebuffer from bind(). Share that owner, rather than moving the renderer
/// out, wrapping every rendering operation, or keeping an untracked pointer.
/// DRM owns the renderer directly. Both variants use one event-loop borrow
/// during protocol dispatch or a frame; no mutex or renderer fork is needed.
///
/// This is an ownership boundary, not a Smithay requirement for Rc/RefCell.
/// A future removal should give WaylandState sole graphics ownership and split
/// renderable scene data from the full protocol-handler state, so a frame can
/// borrow graphics and scene data as disjoint fields. Merely moving graphics
/// into the current state fails: rendering takes both a mutable renderer and
/// mutable WaylandState. Nested bind() must also retain its framebuffer borrow
/// until submission. Keep protocol callbacks outside that frame borrow (see
/// dispatch_pending_commits), rather than taking the owner out temporarily or
/// restoring a raw renderer pointer.
#[derive(Clone)]
pub(crate) enum GraphicsHandle {
    Nested(Rc<RefCell<WinitGraphicsBackend<GlesRenderer>>>),
    Drm(Rc<RefCell<GlesRenderer>>),
}

impl GraphicsHandle {
    pub(crate) fn with_renderer<T>(&self, f: impl FnOnce(&mut GlesRenderer) -> T) -> T {
        match self {
            Self::Nested(backend) => f(backend.borrow_mut().renderer()),
            Self::Drm(renderer) => f(&mut renderer.borrow_mut()),
        }
    }
}

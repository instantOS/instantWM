//! Sole ownership of graphics, borrowed alongside disjoint native scene data.
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::WinitGraphicsBackend;

/// Nested bind() lends a renderer and framebuffer together. Preserve that
/// owner and its borrow through submission; no pointers or runtime borrow checks.
pub(crate) enum Graphics {
    Nested(Box<WinitGraphicsBackend<GlesRenderer>>),
    Drm(Box<GlesRenderer>),
}
impl Graphics {
    pub(crate) fn with_renderer<T>(&mut self, f: impl FnOnce(&mut GlesRenderer) -> T) -> T {
        match self {
            Self::Nested(backend) => f(backend.renderer()),
            Self::Drm(renderer) => f(renderer),
        }
    }
}

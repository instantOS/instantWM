// X11/Wayland backend checks removed; use typed contexts instead.

macro_rules! require_x11_ret {
    ($ctx:expr, $ret:expr) => {
        match $ctx {
            crate::contexts::WmCtx::X11(_) => {}
            crate::contexts::WmCtx::Wayland(_) => return $ret,
        }
    };
}

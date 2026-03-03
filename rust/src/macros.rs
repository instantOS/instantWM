/// Early return if the backend is Wayland (X11-only operations).
#[macro_export]
macro_rules! require_x11 {
    ($ctx:expr) => {
        if $ctx.backend_kind() == $crate::backend::BackendKind::Wayland {
            return;
        }
    };
}

/// Early return with a value if the backend is Wayland.
#[macro_export]
macro_rules! require_x11_ret {
    ($ctx:expr, $ret:expr) => {
        if $ctx.backend_kind() == $crate::backend::BackendKind::Wayland {
            return $ret;
        }
    };
}

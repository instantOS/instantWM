//! Optional libwayland queue sizing without raising the minimum library version.

use smithay::reexports::wayland_server::DisplayHandle;

/// Returns false on libwayland < 1.23, where the default limit is retained.
pub(super) fn set_default_max_buffer_size(display: &DisplayHandle, size: usize) -> bool {
    type SetBufferSize = unsafe extern "C" fn(*mut libc::c_void, libc::size_t);

    // SAFETY: RTLD_NOLOAD looks up the library already held by wayland-backend;
    // it must not load a second libwayland with a different display ABI. The
    // symbol has the signature above since its introduction in libwayland 1.23.
    // The display pointer belongs to that library and remains live throughout
    // the call. Balance our dlopen reference on both symbol lookup paths.
    unsafe {
        let library = libc::dlopen(
            c"libwayland-server.so.0".as_ptr(),
            libc::RTLD_LAZY | libc::RTLD_NOLOAD,
        );
        if library.is_null() {
            log::debug!("libwayland queue sizing unavailable: server library is not loaded");
            return false;
        }

        let symbol = libc::dlsym(library, c"wl_display_set_default_max_buffer_size".as_ptr());
        let available = !symbol.is_null();
        if available {
            let set_size: SetBufferSize = std::mem::transmute(symbol);
            set_size(display.backend_handle().display_ptr().cast(), size);
        } else {
            log::debug!("libwayland queue sizing unavailable: retaining the library default");
        }
        libc::dlclose(library);
        available
    }
}

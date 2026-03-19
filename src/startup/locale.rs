#![allow(clippy::manual_c_str_literals)]
use libc::{setlocale, LC_CTYPE};

pub fn set_locale() -> Result<(), ()> {
    unsafe {
        let result = setlocale(LC_CTYPE, b"\0".as_ptr() as *const libc::c_char);
        if result.is_null() {
            eprintln!("warning: no locale support");
        }
    }
    Ok(())
}

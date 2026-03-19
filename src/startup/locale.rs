use libc::{LC_CTYPE, setlocale};

pub fn set_locale() -> Result<(), ()> {
    unsafe {
        let result = setlocale(LC_CTYPE, c"".as_ptr());
        if result.is_null() {
            eprintln!("warning: no locale support");
        }
    }
    Ok(())
}

// TODO: Port utility functions from util.c

pub fn die(msg: &str) -> ! {
    eprintln!("{}", msg);
    std::process::exit(1);
}

pub fn ecalloc<T>(nmemb: usize) -> Box<[T]> {
    let size = nmemb * std::mem::size_of::<T>();
    if size == 0 {
        die("ecalloc: zero size");
    }
    let mut v: Vec<T> = Vec::with_capacity(nmemb);
    unsafe {
        v.set_len(nmemb);
    }
    v.into_boxed_slice()
}

pub fn min<T: Ord>(a: T, b: T) -> T {
    if a < b {
        a
    } else {
        b
    }
}

pub fn max<T: Ord>(a: T, b: T) -> T {
    if a > b {
        a
    } else {
        b
    }
}

pub fn clamp<T: Ord>(val: T, min_val: T, max_val: T) -> T {
    max(min_val, min(max_val, val))
}

pub fn clean_mask(mask: u32, numlockmask: u32) -> u32 {
    mask & !(numlockmask | x11rb::protocol::xproto::ModMask::LOCK.into())
        & (x11rb::protocol::xproto::ModMask::SHIFT.into()
            | x11rb::protocol::xproto::ModMask::CONTROL.into()
            | x11rb::protocol::xproto::ModMask::M1.into()
            | x11rb::protocol::xproto::ModMask::M2.into()
            | x11rb::protocol::xproto::ModMask::M3.into()
            | x11rb::protocol::xproto::ModMask::M4.into()
            | x11rb::protocol::xproto::ModMask::M5.into())
}

pub fn length<T>(slice: &[T]) -> usize {
    slice.len()
}

pub fn tagmask(num_tags: usize) -> u32 {
    (1 << num_tags) - 1
}

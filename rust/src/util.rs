pub fn die(msg: &str) -> ! {
    eprintln!("{}", msg);
    std::process::exit(1);
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
    let lock_mask: u32 = x11rb::protocol::xproto::ModMask::LOCK.bits() as u32;
    mask & !(numlockmask | lock_mask)
        & (x11rb::protocol::xproto::ModMask::SHIFT.bits() as u32
            | x11rb::protocol::xproto::ModMask::CONTROL.bits() as u32
            | x11rb::protocol::xproto::ModMask::M1.bits() as u32
            | x11rb::protocol::xproto::ModMask::M2.bits() as u32
            | x11rb::protocol::xproto::ModMask::M3.bits() as u32
            | x11rb::protocol::xproto::ModMask::M4.bits() as u32
            | x11rb::protocol::xproto::ModMask::M5.bits() as u32)
}

pub fn length<T>(slice: &[T]) -> usize {
    slice.len()
}

pub fn tagmask(num_tags: usize) -> u32 {
    (1 << num_tags) - 1
}

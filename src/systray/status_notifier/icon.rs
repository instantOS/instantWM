use super::*;

pub(super) fn fetch_item_icon_on_conn(
    conn: &Connection,
    service: &str,
    path: &str,
) -> Option<(Arc<[u8]>, Size)> {
    let proxy = uncached_proxy(conn, service, path, ITEM_IFACE).ok()?;

    let pixmaps: Vec<(i32, i32, Vec<u8>)> = proxy.get_property("IconPixmap").ok()?;
    if pixmaps.is_empty() {
        return None;
    }

    let (size, bytes) = select_largest_valid_pixmap(pixmaps)?;
    let rgba = dbus_icon_bytes_to_rgba(&bytes, size)?;
    Some((Arc::from(rgba), size))
}

pub(super) fn select_largest_valid_pixmap(
    pixmaps: Vec<(i32, i32, Vec<u8>)>,
) -> Option<(Size, Vec<u8>)> {
    pixmaps
        .into_iter()
        .filter_map(|(width, height, bytes)| {
            let pixels = usize::try_from(width)
                .ok()?
                .checked_mul(usize::try_from(height).ok()?)?;
            let required_bytes = pixels.checked_mul(4)?;
            if bytes.len() < required_bytes {
                return None;
            }
            let area = i64::from(width) * i64::from(height);
            Some((area, width, height, bytes))
        })
        .max_by_key(|(area, _, _, _)| *area)
        .map(|(_, width, height, bytes)| (Size::new(width, height), bytes))
}

pub(super) fn dbus_icon_bytes_to_rgba(bytes: &[u8], size: Size) -> Option<Vec<u8>> {
    if !size.is_positive() {
        return None;
    }
    let px_count = (size.w as usize).checked_mul(size.h as usize)?;
    let need = px_count.checked_mul(4)?;
    if bytes.len() < need {
        return None;
    }

    let mut out = vec![0u8; need];
    for i in 0..px_count {
        let si = i * 4;
        // StatusNotifierItem::IconPixmap stores ARGB32 pixels in network byte
        // order, so each pixel arrives as A, R, G, B bytes on the wire.
        let a = bytes[si];
        let r = bytes[si + 1];
        let g = bytes[si + 2];
        let b = bytes[si + 3];
        out[si] = r;
        out[si + 1] = g;
        out[si + 2] = b;
        out[si + 3] = a;
    }
    Some(out)
}

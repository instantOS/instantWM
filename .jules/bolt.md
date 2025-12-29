## 2024-05-23 - Reusing XftDraw Objects
**Learning:** `XftDrawCreate` and `XftDrawDestroy` are called repeatedly in `drw_text` for every text rendering operation (window titles, status bar, tags). This involves memory allocation and setup overhead.
**Action:** Cache the `XftDraw` object in the `Drw` structure, associated with the `Drawable` (pixmap). Only recreate it when the pixmap is resized/recreated. This reduces the overhead of text rendering, which is a frequent operation in window managers.

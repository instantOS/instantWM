# Bolt's Journal

## 2024-05-23 - [Caching XftDraw for Performance]
**Learning:** `drw_text` repeatedly creates and destroys `XftDraw` objects for every text rendering operation. This involves memory allocation and X server round-trips.
**Action:** Cache the `XftDraw` object in the `Drw` structure and reuse it. Recreate it only when the underlying drawable (pixmap) is resized/recreated. This reduces overhead significantly in the hot path of bar drawing.

## 2026-01-15 - XSync vs XFlush in Drawing Path
**Learning:** `drw_map` uses `XSync`, which forces a round-trip to the X server. This is a significant bottleneck for frequent drawing operations like status bar updates.
**Action:** Replace `XSync` with `XFlush` in `drw_map` to improve performance, as `XCopyArea` order is guaranteed by the X protocol for the same client.

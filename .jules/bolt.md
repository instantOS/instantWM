## 2026-01-19 - [Stack Allocation in drawstatusbar]
**Learning:** `drawstatusbar` in `bar.c` uses `malloc` for a temporary string buffer on every redraw. This is a hot path. The source string `stext` is a global fixed-size buffer of 1024 bytes.
**Action:** Replace `malloc` with a stack buffer `char text[1024]` to avoid heap allocation overhead and potential fragmentation. This is safe because the input size is bounded by the global `stext` size.

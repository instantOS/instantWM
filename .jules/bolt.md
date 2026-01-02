## 2026-01-02 - XSync in Animation Loops
**Learning:** `XSync` forces a round-trip to the X server, blocking execution until the server catches up. Using it inside `resizeclient`, which is called repeatedly during animations, caused significant performance degradation.
**Action:** Replace `XSync` with `XFlush` (or rely on implicit flushing) in high-frequency X11 calls like animations or window resizing to avoid blocking.

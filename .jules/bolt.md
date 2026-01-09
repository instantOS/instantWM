## 2026-01-09 - X11 Synchronization Bottlenecks
**Learning:** `XSync` forces a round-trip to the X server, blocking the client until the server responds. In `drw_map` (used for bar drawing) and `resizeclient` (used for window resizing), this adds significant latency.
**Action:** Replace `XSync` with `XFlush` in rendering and non-dependent configuration paths. `XFlush` sends the request buffer without waiting, which is sufficient for operations like drawing or configuring windows where the client doesn't immediately read back the result.

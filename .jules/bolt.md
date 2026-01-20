## 2026-01-20 - XSync vs XFlush in Event-Driven Drawing
**Learning:** `XSync` is a blocking round-trip to the X server, while `XFlush` is asynchronous. In `instantwm`, `XSync` was used in `drw_map` (called on every bar redraw) and `resizeclient` (called on every window resize), causing significant performance degradation due to unnecessary waiting for server confirmation.
**Action:** Replace `XSync` with `XFlush` in high-frequency drawing/window management paths when immediate confirmation is not strictly required for logic correctness.

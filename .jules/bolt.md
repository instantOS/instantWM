# Bolt's Journal

## 2024-05-22 - X11 Performance Strategy
**Learning:** In X11 window managers, `XSync` is a blocking call that forces a round-trip to the X server, waiting for all requests to be processed. This is extremely slow compared to `XFlush`, which just sends the buffer.
**Action:** Only use `XSync` when absolutely necessary (e.g., when you need to guarantee an event has been processed before continuing, or for debugging). Prefer `XFlush` for general updates.

## 2024-05-22 - Optimize Blocking XSync Calls
**Learning:** Replacing blocking `XSync` calls with `XFlush` in high-frequency paths (like drawing `drw_map` and resizing `resizeclient`) significantly improves responsiveness. `XSync` is still useful in `restack` to ensure stacking order is applied before checking events, but generally should be avoided in rendering loops.
**Action:** Audit `XSync` usage. Replace with `XFlush` where immediate server confirmation isn't strictly required.

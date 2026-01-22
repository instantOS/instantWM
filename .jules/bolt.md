## 2024-05-23 - XSync vs XFlush in Drawing Paths
**Learning:** Found `XSync` being used in `drw_map`, causing blocking round-trips to the X server for every buffer swap. This defeats the purpose of double buffering and significantly reduces rendering throughput.
**Action:** Replace `XSync` with `XFlush` in pure drawing functions. Only use `XSync` when synchronization or error handling is explicitly required.

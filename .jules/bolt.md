
## 2026-02-03 - XSync in Drawing Loops
**Learning:** `XSync` forces a blocking round-trip to the X server, which is catastrophic for performance in frequently called drawing functions like `drw_map`.
**Action:** Replace `XSync` with `XFlush` in drawing routines where strict synchronization is not required.

## 2026-02-04 - XSync vs XFlush in Drawing
**Learning:**  forces a blocking round-trip to the X server, which is catastrophic for frequent drawing operations like bar updates ().
**Action:** Use  for drawing operations where the client doesn't need to wait for the server's response.
## 2026-02-04 - XSync vs XFlush in Drawing
**Learning:** `XSync` forces a blocking round-trip to the X server, which is catastrophic for frequent drawing operations like bar updates (`drw_map`).
**Action:** Use `XFlush` for drawing operations where the client doesn't need to wait for the server's response.

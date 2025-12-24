## 2025-12-24 - XSync Blocking in Critical Paths
**Learning:** XSync is a blocking call that waits for the X server to process all requests. In performance-critical paths like drawing (drw_map) and resizing (resizeclient), this introduces significant latency (round-trip time).
**Action:** Use XFlush instead of XSync for screen updates where we don't need to wait for a reply. XFlush sends the requests but returns immediately.

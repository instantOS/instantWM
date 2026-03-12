When I look at `i3status-rs` output, it seems it sends an array of blocks:
```json
{"version":1,"click_events":true}
[
[{"name":"time","instance":"time","full_text":" 12:34 "}]
```

Wait, what if `i3status-rs` output has `{"version":1}` (with spaces or other fields)?
```rust
                    if trim_line == "["
                        || trim_line.starts_with("{\"version\"")
                        || trim_line.is_empty()
```
If it is `{"version": 1}` then `starts_with("{\"version\"")` won't match (due to the space). It will be sent to the status bar, and the bar will try to parse `{"version": 1}`. It's not a list, so `serde_json` fails to parse as `Vec<RawI3Block>`. Then it treats it as raw string and prints it!
Oh, wait!

Let's test this!

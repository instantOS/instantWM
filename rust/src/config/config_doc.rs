pub const CONFIG_DOC: &str = r##"Use $XDG_CONFIG_HOME/instantwm/config.toml (or ~/.config/instantwm/config.toml).

Top-level keys:
  fonts = ["Font Name:size=12", "Fallback Font:size=12"]

Colors section:
  [colors.tag.normal], [colors.tag.hover]
  [colors.window.normal], [colors.window.hover]
  [colors.close_button.normal], [colors.close_button.hover]
  [colors.border]
  [colors.status]

Each tag/window/close_button section contains per-scheme triplets:
  inactive, filled, focus, nofocus, empty
  focus, normal, minimized, sticky, sticky_focus, overlay, overlay_focus
  normal, locked, fullscreen

Triplet syntax:
  focus = { fg = "#RRGGBB", bg = "#RRGGBB", detail = "#RRGGBB" }

Example (truncated):
[colors.tag.normal]
inactive = { fg = "#DFDFDF", bg = "#121212", detail = "#121212" }
filled = { fg = "#DFDFDF", bg = "#384252", detail = "#89B3F7" }

[colors.border]
normal = "#384252"
tile_focus = "#89B3F7"
float_focus = "#81c995"
snap = "#fdd663"
"##;

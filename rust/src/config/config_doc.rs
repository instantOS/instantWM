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

Keybinds section:
  [[keybinds]]          — override/add normal-mode keybinds
  [[desktop_keybinds]]  — override/add desktop-mode keybinds (no client focused)

  Each entry has:
    modifiers = ["Super", "Shift"]   — modifier keys (Super/Mod, Shift, Ctrl, Alt)
    key = "Return"                   — key name (letters, digits, Return, space, F1-F12, etc.)
    action = "zoom"                  — named action string, OR structured action:
    action = { spawn = ["cmd", "arg"] }
    action = { set_layout = "tile" }
    action = { focus_stack = "next" }
    action = { set_mfact = 0.05 }
    action = { inc_nmaster = 1 }
    action = { unbind = true }       — removes the default binding for this combo

  Named actions include:
    zoom, kill, shut_kill, quit,
    focus_next, focus_prev, focus_last, focus_up/down/left/right,
    toggle_layout, layout_tile, layout_float, layout_monocle, layout_grid,
    cycle_layout_next, cycle_layout_prev, inc_nmaster, dec_nmaster,
    mfact_grow, mfact_shrink,
    center_window, toggle_maximized, distribute_clients,
    key_resize_up/down/left/right, push_up, push_down,
    last_view, follow_view, scroll_left, scroll_right,
    move_client_left, move_client_right, shift_tag_left, shift_tag_right,
    shift_view_left, shift_view_right, view_all, tag_all,
    toggle_overview, toggle_fullscreen_overview,
    focus_mon_prev, focus_mon_next, follow_mon_prev, follow_mon_next,
    set_overlay, create_overlay, scratchpad_toggle, scratchpad_make,
    toggle_bar, toggle_sticky, toggle_alt_tag, toggle_animated,
    toggle_show_tags, toggle_prefix, toggle_fake_fullscreen,
    redraw_win, unhide_all, hide, draw_window, begin_keyboard_move

Example (truncated):
[colors.tag.normal]
inactive = { fg = "#DFDFDF", bg = "#121212", detail = "#121212" }
filled = { fg = "#DFDFDF", bg = "#384252", detail = "#89B3F7" }

[colors.border]
normal = "#384252"
tile_focus = "#89B3F7"
float_focus = "#81c995"
snap = "#fdd663"

[[keybinds]]
modifiers = ["Super"]
key = "Return"
action = { spawn = ["alacritty"] }

[[keybinds]]
modifiers = ["Super", "Shift"]
key = "q"
action = { unbind = true }
"##;

This is a Rust rewrite of a C dwm fork. It has since deviated a lot from dwm,
and added a Wayland backend in addition to X11. Backends should be abstracted
away, so that we can support both X11 and Wayland without much effort. 

Do not do release builds as they take too much time and do not yield
improvements when just testing

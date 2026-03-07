Check that the accessor patterns for X11 and Core and Wayland details are not
messy or have tons of different ways to do it.

If you find a mess, consolidate on one pattern. I don't care how many callsites
need to be updated, just think hard about which pattern is the best

Match arms with x11 and wayland which are a no-op for one of the two are an
indicator that something should change there. The function is likely an X11 or
Wayland implementation detail, and as such should be able to take only an X11 or
Wayland specific context object. If that is not possible, meaning the function
also does backend agnostic stuff then the function has too many
responsibilities, and should be refactored. Same goes for its callers. Highly
X11 and wayland specific details should probably go into their respective
backend modules. X11 should have a folder in that, just like wayland does.

What is reborrow() ? How is it used? Is it a code-smell

Investigate the DRM wayland backend. The wayland compositor works somewhat well
as a nested compositor, is the standalone mode missing something to get it to
work as well? Is there anything unfinished?

Investigate what numlockmask is, why it is all over the place, including
dangerously close to wayland, and if we can make it cleaner, not relying on the
runtime check panic accessor for the X11 stuff.

I would like more encapsulation, that way state could maybe eventually be
propagated cleaner than passing around huge objects all the time.

creating new contexts on the fly is a code smell, investigate some of these
cases and do what is necessary to get the data flow to be clean and readable. No
lazy fixes, things like these are indicative of a deeper issue.

```
let selmon_id = core.g.selected_monitor_id();
crate::layouts::arrange(
    &mut crate::contexts::WmCtx::X11(crate::contexts::WmCtxX11 {
        core: core.reborrow(),
        backend: crate::backend::BackendRef::from_x11(x11.conn, x11.screen_num),
        x11: crate::backend::x11::X11BackendRef::new(x11.conn, x11.screen_num),
        x11_runtime,
        systray: None,
    }),
```





the indicator light for the capslock key does not work on wayland instantwm. 


I would like a swapescape option similar to sway and i3 in instantwm wayland,
also accessible via the IPC. 


Super + Shift + Space does not do anything on wayland.

Super + Left click drag and Super + Right click drag do not work on wayland, but
dragging from the top bar does work




The old xsetroot way of setting the status bar text is outdated, I want a
unified solution for both X11 and wayland. Ideally that soludion can render the
same as i3blocks-rs (look up the format, I also have that installed for you to
test. do run with timeout though)


dunst does not work on the wayland backend, some protocol is missing




investigate this for overdraw issues. Is that a performance risk?




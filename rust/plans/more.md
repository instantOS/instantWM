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
unified solution for both X11 and wayland. Ideally that solution can render the
i3status-rs format. 




investigate this for overdraw issues. Is that a performance risk?

dmenu does not grab input on the wayland backend, I need to click it in order to type in it. This is
not how it works on X11, investigate and fix

on wayland, after I switch tags no window is in focus. Investigate and fix.
Also, clicking on a window should probably focus it, although it should not raise it,
raise only happens when I move it or make it floating or resize it etc. 

clicking a window title should raise that window. 

I would like to be able to configure mouse sensitivity in the wayland backend,
using the toml file and IPC. I want this to work like it does on sway (as
similar as possible), so users don't have to too much new stuff. 



There are a lot of different resize functions which largely do the same thing.
Maybe instead, there should be one, with a resizeoptions struct?
This could have fields like 'respect size hints', 'keep aspect ratio',
'resizedirection' (existing enum) and all the nice stuff. That should also give
the ability to make X11 and Wayland resizing more similar or even share code. 
I am aware this is a potentially big refactor, but it should simplify things. 



The systray does not show up on wayland on the drm backend. I start the
compositor using gdm, open Telegram and there is nothing to the right of the
window title where the telegram icon should be in the systray. Maybe this works
differently between the nested wayland backend and the drm backend. They should
share code if possible. Investigate and fix. 


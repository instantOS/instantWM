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


investigate this for overdraw issues. Is that a performance risk?

dmenu does not grab input on the wayland backend, I need to click it in order to type in it. This is
not how it works on X11, investigate and fix

on wayland, after I switch tags no window is in focus. Investigate and fix.
Also, clicking on a window should probably focus it, although it should not raise it,
raise only happens when I move it or make it floating or resize it etc.

clicking a window title should raise that window.

I would like to be able to configure mouse sensitivity in the wayland backend,
using the toml file and IPC (instantwmctl currently does not have a mouse
subcommand, I would like one with appropriate UX/DX).
I want this to work like it does on sway (as
similar as possible), so users don't have to too much new stuff.



figure out where to put client state, monitor? Monitor mnanager? Something
erlse? Is there a mess?


Pattern 1: Default then require
// backend/x11/lifecycle.rs:267
let isfixed = g.clients.get(&w).map(|c| c.isfixed).unwrap_or(false);  // defaults to false
let mut should_raise = false;
if let Some(client) = g.clients.get_mut(&w) {  // but client MUST exist later
    // ...
}
This is logically inconsistent - if we require the client later, we should require it upfront.
Pattern 2: Silently ignore missing clients
// floating/snap.rs:160-170
if ctx.core.g.clients.get(&win).map(|c| c.isfloating).unwrap_or(false) {
    if let Some(client) = ctx.core.g.clients.get_mut(&win) {  // silently does nothing if client gone
        client.float_geo = client.geo;
    }
}



make keybinds toml configurable
make default layout toml configurable
ability to set keybind to none with toml


I would like the `instantwmctl monitor` command to be able to change
resolution/refresh rate at runtime. I also want this to be configurable through
toml if that is not already the case. 

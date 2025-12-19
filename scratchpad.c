/* See LICENSE file for copyright and license details. */

#include <string.h>

#include "floating.h"
#include "focus.h"
#include "globals.h"
#include "layouts.h"
#include "scratchpad.h"

/* External declarations for variables defined in instantwm.c */
extern const char *termscratchcmd[];

// Update scratchvisible flag for a monitor
void updatescratchvisible(Monitor *m) {
    Client *c;
    m->scratchvisible = 0;
    for (c = m->clients; c; c = c->next) {
        if ((c->tags & SCRATCHPAD_MASK) && c->issticky) {
            m->scratchvisible = 1;
            break;
        }
    }
}

// Find a client with class scratchpad_<name>
Client *findnamedscratchpad(const char *name) {
    if (!name || strlen(name) == 0) {
        return NULL;
    }

    char fullclass[256];
    snprintf(fullclass, sizeof(fullclass), "scratchpad_%s", name);

    Client *c;
    Monitor *m;
    XClassHint ch = {NULL, NULL};

    for (m = mons; m; m = m->next) {
        for (c = m->clients; c; c = c->next) {
            XGetClassHint(dpy, c->win, &ch);
            if (ch.res_class && strcmp(ch.res_class, fullclass) == 0) {
                if (ch.res_class) {
                    XFree(ch.res_class);
                }
                if (ch.res_name) {
                    XFree(ch.res_name);
                }
                return c;
            }
            if (ch.res_name && strcmp(ch.res_name, fullclass) == 0) {
                if (ch.res_class) {
                    XFree(ch.res_class);
                }
                if (ch.res_name) {
                    XFree(ch.res_name);
                }
                return c;
            }
            if (ch.res_class) {
                XFree(ch.res_class);
            }
            if (ch.res_name) {
                XFree(ch.res_name);
            }
        }
    }
    return NULL;
}

void makescratchpad(const Arg *arg) {
    const char *name = arg ? arg->v : NULL;
    if (!name || strlen(name) == 0) {
        return;
    }

    Client *c = findnamedscratchpad(name);
    if (!c) {
        return;
    }

    // Move to scratchpad tag (tag 21)
    c->tags = SCRATCHPAD_MASK;
    c->issticky = 0;

    // Make it float if not already
    setfloating(c, 0);

    focus(NULL);
    arrange(c->mon);
}

void togglescratchpad(const Arg *arg) {
    Client *c;
    Client *found = NULL;
    int handled_action = 0;
    const char *name = arg ? arg->v : NULL;

    if (&overviewlayout == tiling_layout_func(selmon)) {
        return;
    }

    // Handle named scratchpad
    if (name && strlen(name) > 0) {
        Client *named = findnamedscratchpad(name);
        if (named) {
            if (named->issticky) {
                // Hide the named scratchpad
                named->issticky = 0;
                named->tags = SCRATCHPAD_MASK;
                focus(NULL);
                arrange(named->mon);
            } else {
                // Show the named scratchpad
                named->issticky = 1;
                setfloating(named, 0);
                // Move to current monitor if on different monitor
                if (named->mon != selmon) {
                    detach(named);
                    detachstack(named);
                    named->mon = selmon;
                    attach(named);
                    attachstack(named);
                }
                focus(named);
                arrange(selmon);
                restack(selmon);
                if (focusfollowsmouse) {
                    warp_cursor_to_client(named);
                }
            }

            updatescratchvisible(named->mon);
        }
        return;
    }

    // Default behavior for generic scratchpad
    if (selmon->sel && (selmon->sel->tags & (1 << 20))) {
        c = selmon->sel;
        c->issticky = 0;
        c->tags = SCRATCHPAD_MASK;
        selmon->activescratchpad = c;
        focus(NULL);
        arrange(selmon);
        handled_action = 1;
    } else {
        for (c = selmon->clients; c; c = c->next) {
            if ((c->tags & SCRATCHPAD_MASK) && c->issticky) {
                focus(c);
                restack(selmon);
                handled_action = 1;
                break;
            }
        }

        if (!handled_action) {
            if (selmon->activescratchpad &&
                (selmon->activescratchpad->tags & (1 << 20))) {
                found = selmon->activescratchpad;
            }

            if (!found) {
                for (c = selmon->clients; c; c = c->next) {
                    if ((c->tags & SCRATCHPAD_MASK) && !c->issticky) {
                        found = c;
                        break;
                    }
                }
            }

            if (found) {
                found->issticky = 1;
                setfloating(found, 0);
                focus(found);
                arrange(selmon);
                restack(selmon);
                if (focusfollowsmouse) {
                    warp_cursor_to_client(found);
                }
                selmon->activescratchpad = found;
                handled_action = 1;
            } else {
                spawn(&((Arg){.v = termscratchcmd}));
                handled_action = 1;
            }
        }
    }

    if (handled_action) {
        updatescratchvisible(selmon);
    }
}

void createscratchpad(const Arg *arg) {
    Client *c;
    if (!selmon->sel) {
        return;
    }
    c = selmon->sel;

    // turn scratchpad back into normal window
    if (c->tags == SCRATCHPAD_MASK) {
        tag(&((Arg){.ui = 1 << (selmon->pertag->current_tag - 1)}));

        updatescratchvisible(selmon);
        return;
    }

    c->tags = SCRATCHPAD_MASK;
    c->issticky = 0;
    if (!c->isfloating) {
        toggle_floating(NULL);
    }

    focus(NULL);
    arrange(selmon);
}

void showscratchpad(const Arg *arg) {
    const char *name = arg ? arg->v : NULL;

    // Handle named scratchpad
    if (name && strlen(name) > 0) {
        Client *named = findnamedscratchpad(name);
        if (named) {
            named->issticky = 1;
            setfloating(named, 0);
            // Move to current monitor if on different monitor
            if (named->mon != selmon) {
                detach(named);
                detachstack(named);
                named->mon = selmon;
                attach(named);
                attachstack(named);
            }
            focus(named);
            arrange(selmon);
            restack(selmon);
            if (focusfollowsmouse) {
                warp_cursor_to_client(named);
            }

            updatescratchvisible(named->mon);
        }
        return;
    }

    // Default behavior
    if (selmon->scratchvisible) {
        Client *c;
        for (c = selmon->clients; c; c = c->next) {
            if ((c->tags & SCRATCHPAD_MASK) && c->issticky) {
                focus(c);
                restack(selmon);
                if (focusfollowsmouse) {
                    warp_cursor_to_client(c);
                }
                return;
            }
        }
    } else {
        togglescratchpad(NULL);
    }
}

void hidescratchpad(const Arg *arg) {
    const char *name = arg ? arg->v : NULL;

    // Handle named scratchpad
    if (name && strlen(name) > 0) {
        Client *named = findnamedscratchpad(name);
        if (named && named->issticky) {
            named->issticky = 0;
            named->tags = SCRATCHPAD_MASK;
            focus(NULL);
            arrange(named->mon);

            updatescratchvisible(named->mon);
        }
        return;
    }

    // Default behavior: hide all visible scratchpads
    Client *c;
    int changed = 0;
    for (c = selmon->clients; c; c = c->next) {
        if ((c->tags & SCRATCHPAD_MASK) && c->issticky) {
            c->issticky = 0;
            c->tags = SCRATCHPAD_MASK;
            changed = 1;
        }
    }
    if (changed) {
        selmon->scratchvisible = 0;
        focus(NULL);
        arrange(selmon);
    }
}

void scratchpadstatus(const Arg *arg) {
    char status[32];
    const char *name = arg ? arg->v : NULL;

    // If named scratchpad requested, check its specific visibility
    if (name && strlen(name) > 0) {
        Client *named = findnamedscratchpad(name);
        int visible = named && named->issticky;
        snprintf(status, sizeof(status), "ipc:scratchpad:%d", visible);
    } else {
        // Default behavior: check if any scratchpad is visible on current
        // monitor
        snprintf(status, sizeof(status), "ipc:scratchpad:%d",
                 selmon->scratchvisible);
    }
    XStoreName(dpy, root, status);
    XFlush(dpy);
}

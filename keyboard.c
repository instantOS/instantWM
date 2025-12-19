/* See LICENSE file for copyright and license details. */

#include <X11/Xlib.h>

#include "bar.h"
#include "client.h"
#include "floating.h"
#include "focus.h"
#include "globals.h"
#include "instantwm.h"
#include "keyboard.h"
#include "layouts.h"
#include "overlay.h"

/* External declarations not covered by headers */
extern int freealttab;

#define CLEANMASK(mask)                                                        \
    (mask & ~(numlockmask | LockMask) &                                        \
     (ShiftMask | ControlMask | Mod1Mask | Mod2Mask | Mod3Mask | Mod4Mask |    \
      Mod5Mask))

void keyrelease(XEvent *e) { (void)e; /* unused */ }

void grabkeys(void) {
    updatenumlockmask();
    {
        unsigned int i;
        unsigned int j;
        unsigned int k;
        unsigned int modifiers[] = {0, LockMask, numlockmask,
                                    numlockmask | LockMask};
        int start;
        int end;
        int skip;
        KeySym *syms;

        XUngrabKey(dpy, AnyKey, AnyModifier, root);
        XDisplayKeycodes(dpy, &start, &end);
        syms = XGetKeyboardMapping(dpy, start, end - start + 1, &skip);
        if (!syms) {
            return;
        }

        for (k = start; k <= (unsigned int)end; k++) {
            /* Skip invalid keycodes to prevent X11 BadValue errors */
            if (k > 255) {
                continue;
            }

            for (i = 0; i < keys_len; i++) {
                if (keys[i].keysym == syms[(k - start) * skip]) {
                    for (j = 0; j < 4; j++) {
                        if (freealttab && keys[i].mod == Mod1Mask) {
                            continue;
                        }
                        XGrabKey(dpy, k, keys[i].mod | modifiers[j], root, True,
                                 GrabModeAsync, GrabModeAsync);
                    }
                }
            }

            /* add keyboard shortcuts without modifiers when tag is empty */
            if (!selmon->sel) {
                for (i = 0; i < dkeys_len; i++) {
                    if (dkeys[i].keysym == syms[(k - start) * skip]) {
                        for (j = 0; j < 4; j++) {
                            XGrabKey(dpy, k, dkeys[i].mod | modifiers[j], root,
                                     True, GrabModeAsync, GrabModeAsync);
                        }
                    }
                }
            }
        }

        XFree(syms);
    }
}

void keypress(XEvent *e) {
    unsigned int i;
    KeySym keysym;
    XKeyEvent *ev;

    ev = &e->xkey;
    keysym = XKeycodeToKeysym(dpy, (KeyCode)ev->keycode, 0);
    for (i = 0; i < keys_len; i++) {
        if (keysym == keys[i].keysym &&
            CLEANMASK(keys[i].mod) == CLEANMASK(ev->state) && keys[i].func) {
            keys[i].func(&(keys[i].arg));
        }
    }

    if (!selmon->sel) {
        for (i = 0; i < dkeys_len; i++) {
            if (keysym == dkeys[i].keysym &&
                CLEANMASK(dkeys[i].mod) == CLEANMASK(ev->state) &&
                dkeys[i].func) {
                dkeys[i].func(&(dkeys[i].arg));
            }
        }
    }
}

void uppress(const Arg *arg) {
    (void)arg; /* unused */
    if (!selmon->sel) {
        return;
    }
    if (selmon->sel == selmon->overlay) {
        setoverlaymode(0);
        return;
    }
    if (selmon->sel->isfloating) {
        toggle_floating(NULL);
        return;
    }
    hide(selmon->sel);
}

void downpress(const Arg *arg) {
    (void)arg; /* unused */
    if (unhideone()) {
        return;
    }
    if (!selmon->sel) {
        return;
    }

    if (selmon->sel->snapstatus) {
        resetsnap(selmon->sel);
        return;
    }

    if (selmon->sel == selmon->overlay) {
        setoverlaymode(OverlayBottom);
        return;
    }
    if (!selmon->sel->isfloating) {
        toggle_floating(NULL);
        return;
    }
}

void upkey(const Arg *arg) {
    if (!selmon->sel) {
        return;
    }

    if (&overviewlayout == tiling_layout_func(selmon)) {
        direction_focus(&((Arg){.ui = 0}));
        return;
    }

    if (NULL == tiling_layout_func(selmon)) {
        Client *c;
        c = selmon->sel;
        XSetWindowBorder(dpy, c->win,
                         borderscheme[SchemeBorderTileFocus].pixel);
        changesnap(c, 0);
        return;
    }
    focusstack(arg);
}

void downkey(const Arg *arg) {
    if (&overviewlayout == tiling_layout_func(selmon)) {
        direction_focus(&((Arg){.ui = 2}));
        return;
    }

    if (NULL == tiling_layout_func(selmon)) {
        if (!selmon->sel) {
            return;
        }
        /* unmaximize */
        changesnap(selmon->sel, 2);
        return;
    }
    focusstack(arg);
}

void spacetoggle(const Arg *arg) {
    if (NULL == tiling_layout_func(selmon)) {
        if (!selmon->sel) {
            return;
        }
        Client *c;
        c = selmon->sel;

        if (c->snapstatus) {
            resetsnap(c);
        } else {
            XSetWindowBorder(dpy, selmon->sel->win,
                             borderscheme[SchemeBorderTileFocus].pixel);
            savefloating(c);
            selmon->sel->snapstatus = SnapMaximized;
            arrange(selmon);
        }
    } else {
        toggle_floating(arg);
    }
}

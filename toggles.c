/* See LICENSE file for copyright and license details. */

#include "toggles.h"
#include "bar.h"
#include "floating.h"
#include "globals.h"
#include "overlay.h"
#include "push.h"

/* extern declarations for variables from instantwm.c */
extern int freealttab;

void ctrltoggle(int *value, int arg) {
    if (arg == 0 || arg == 2) {
        *value = !*value;
    } else {
        if (arg == 1) {
            *value = 0;
        } else {
            *value = 1;
        }
    }
}

// toggle tag icon view
void togglealttag(const Arg *arg) {
    ctrltoggle(&showalttag, arg->ui);

    Monitor *m;
    for (m = mons; m; m = m->next) {
        drawbar(m);
    }

    tagwidth = gettagwidth();
}

void alttabfree(const Arg *arg) {
    ctrltoggle(&freealttab, arg->ui);
    grabkeys();
}

// make client show on all tags
void togglesticky(const Arg *arg) {
    if (!selmon->sel) {
        return;
    }
    selmon->sel->issticky = !selmon->sel->issticky;
    arrange(selmon);
}

void toggleprefix(const Arg *arg) {
    tagprefix ^= 1;
    drawbar(selmon);
}

// disable/enable animations
void toggleanimated(const Arg *arg) { ctrltoggle(&animated, arg->ui); }

void setborderwidth(const Arg *arg) {
    Client *c;
    int width;
    int d;
    if (!selmon->sel) {
        return;
    }
    c = selmon->sel;
    width = c->border_width;
    c->border_width = arg->i;
    d = width - c->border_width;
    resize(c, c->x, c->y, c->w + 2 * d, c->h + 2 * d, 0);
}

// disable/enable window focus following the mouse
void togglefocusfollowsmouse(const Arg *arg) {
    ctrltoggle(&focusfollowsmouse, arg->ui);
}

// disable/enable window focus following the mouse
void togglefocusfollowsfloatmouse(const Arg *arg) {
    ctrltoggle(&focusfollowsfloatmouse, arg->ui);
}

// double the window refresh rate
void toggledoubledraw(const Arg *arg) { doubledraw = !doubledraw; }

/* togglefakefullscreen() stays in instantwm.c - depends on config.h */

// lock prevents windows from getting closed until unlocked
void togglelocked(const Arg *arg) {
    if (!selmon->sel) {
        return;
    }
    selmon->sel->islocked = !selmon->sel->islocked;
    drawbar(selmon);
}

// toggle vacant tags
void toggleshowtags(const Arg *arg) {
    int showtags = selmon->showtags;
    ctrltoggle(&showtags, arg->ui);
    selmon->showtags = showtags;
    tagwidth = gettagwidth();
    drawbar(selmon);
}

/* togglebar() stays in instantwm.c - depends on config.h */

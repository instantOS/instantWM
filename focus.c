/* See LICENSE file for copyright and license details. */

#include "focus.h"
#include "scratchpad.h"
#include "tags.h"

/* External declarations for variables defined in instantwm.c */
extern Display *dpy;
extern Monitor *selmon;
extern Monitor *mons;
extern Window root;
extern int bh;
extern Client *lastclient;

/* External function declarations */
extern int getrootptr(int *x, int *y);
extern void focus(Client *c);
extern void unfocus(Client *c, int setfocus);
extern void restack(Monitor *m);

void directionfocus(const Arg *arg) {
    Client *c;
    Client *sc;
    Client *outclient = NULL;
    Monitor *m;
    int minscore;
    int score;
    int foundone = 0;
    int direction = arg->ui;

    if (!selmon->sel)
        return;
    m = selmon;
    sc = selmon->sel;
    minscore = 0;

    int cx, cy;
    int sx, sy;
    sx = sc->x + (sc->w / 2);
    sy = sc->y + (sc->h / 2);

    for (c = m->clients; c; c = c->next) {
        if (!(ISVISIBLE(c)))
            continue;

        cx = c->x + (c->w / 2);
        cy = c->y + (c->h / 2);

        if (c == sc || (direction == 0 && cy > sy) ||
            (direction == 1 && cx < sx) || (direction == 2 && cy < sy) ||
            (direction == 3 && cx > sx))
            continue;

        if (direction % 2 == 0) {
            score = abs(sx - cx) + abs(sy - cy) / 4;
            if (abs(sx - cx) > abs(sy - cy))
                continue;
        } else {
            score = abs(sy - cy) + abs(sx - cx) / 4;
            if (abs(sy - cy) > abs(sx - cx))
                continue;
        }

        if (score < minscore || minscore == 0) {
            outclient = c;
            foundone = 1;
            minscore = score;
        }
    }
    if (outclient && foundone) {
        focus(outclient);
    }
}

void focuslastclient(const Arg *arg) {
    Client *c;

    if (!lastclient)
        return;

    c = lastclient;

    if (c->tags & 1 << 20) {
        togglescratchpad(NULL);
        return;
    }

    const Arg a = {.ui = c->tags};
    if (selmon != c->mon) {
        unfocus(selmon->sel, 0);
        selmon = c->mon;
    }

    if (selmon->sel)
        lastclient = selmon->sel;

    view(&a);
    focus(c);
    restack(selmon);
}

void warp(const Client *c) {
    int x, y;

    if (!c) {
        XWarpPointer(dpy, None, root, 0, 0, 0, 0, selmon->wx + selmon->ww / 2,
                     selmon->wy + selmon->wh / 2);
        return;
    }

    if (!getrootptr(&x, &y) ||
        (x > c->x - c->bw && y > c->y - c->bw && x < c->x + c->w + c->bw * 2 &&
         y < c->y + c->h + c->bw * 2) ||
        (y > c->mon->by && y < c->mon->by + bh) || (c->mon->topbar && !y))
        return;

    XWarpPointer(dpy, None, c->win, 0, 0, 0, 0, c->w / 2, c->h / 2);
}

void forcewarp(const Client *c) {
    XWarpPointer(dpy, None, c->win, 0, 0, 0, 0, c->w / 2, 10);
}

void warpinto(const Client *c) {
    int x, y;
    getrootptr(&x, &y);
    if (x < c->x)
        x = c->x + 10;
    else if (x > c->x + c->w)
        x = c->x + c->w - 10;

    if (y < c->y)
        y = c->y + 10;
    else if (y > c->y + c->h)
        y = c->y + c->h - 10;
    XWarpPointer(dpy, None, root, 0, 0, 0, 0, x, y);
}

void warpfocus() { warp(selmon->sel); }

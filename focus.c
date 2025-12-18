/* See LICENSE file for copyright and license details. */

#include "focus.h"
#include "client.h"
#include "globals.h"
#include "instantwm.h"
#include "scratchpad.h"
#include "tags.h"

/* External declarations not covered by headers */
extern Client *lastclient;

void direction_focus(const Arg *arg) {
    Client *c;
    Client *source; /* The window we're navigating from */
    Client *out_client = NULL;
    Monitor *m;
    int min_score;
    int score;
    int found_one = 0;
    int direction = arg->ui;

    if (!selmon->sel)
        return;
    m = selmon;
    source = selmon->sel;
    min_score = 0;

    int cx, cy;
    int sx, sy;
    sx = source->x + (source->w / 2);
    sy = source->y + (source->h / 2);

    for (c = m->clients; c; c = c->next) {
        if (!(ISVISIBLE(c)))
            continue;

        cx = c->x + (c->w / 2);
        cy = c->y + (c->h / 2);

        /* Skip windows that are in the wrong direction from source */
        if (c == source || (direction == FocusDirUp && cy > sy) ||
            (direction == FocusDirRight && cx < sx) ||
            (direction == FocusDirDown && cy < sy) ||
            (direction == FocusDirLeft && cx > sx))
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

        if (score < min_score || min_score == 0) {
            out_client = c;
            found_one = 1;
            min_score = score;
        }
    }
    if (out_client && found_one) {
        focus(out_client);
    }
}

void focus_last_client(const Arg *arg) {
    Client *c;

    if (!lastclient)
        return;

    c = lastclient;

    if (c->tags & SCRATCHPAD_MASK) {
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

void warp_cursor_to_client(const Client *c) {
    int x, y;

    if (!c) {
        XWarpPointer(dpy, None, root, 0, 0, 0, 0, selmon->wx + selmon->ww / 2,
                     selmon->wy + selmon->wh / 2);
        return;
    }

    if (!getrootptr(&x, &y) ||
        (x > c->x - c->border_width && y > c->y - c->border_width &&
         x < c->x + c->w + c->border_width * 2 &&
         y < c->y + c->h + c->border_width * 2) ||
        (y > c->mon->by && y < c->mon->by + bh) || (c->mon->topbar && !y))
        return;

    XWarpPointer(dpy, None, c->win, 0, 0, 0, 0, c->w / 2, c->h / 2);
}

void force_warp(const Client *c) {
    XWarpPointer(dpy, None, c->win, 0, 0, 0, 0, c->w / 2, 10);
}

/**
 * Warps the cursor into the client window if it is currently outside.
 *
 * Ensures the cursor is at least 10 pixels inside the window boundaries.
 * If the cursor is already inside, it remains untouched.
 *
 * @param c The client window to warp into.
 */
void warp_into(const Client *c) {
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

void warp_to_focus() { warp_cursor_to_client(selmon->sel); }

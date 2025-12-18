/* See LICENSE file for copyright and license details. */

#include <unistd.h>

#include "animation.h"
#include "floating.h"
#include "globals.h"
#include "layouts.h"
#include "tags.h"

double easeOutCubic(double t) {
    t--;
    return 1 + t * t * t;
}

void checkanimate(Client *c, int x, int y, int w, int h, int frames,
                  int resetpos) {
    if (c->x == x && c->y == y && c->w == w && c->h == h) {
        return;
    } else {
        animateclient(c, x, y, w, h, frames, resetpos);
    }
}

// move client to position within a set amount of frames
void animateclient(Client *c, int x, int y, int w, int h, int frames,
                   int resetpos) {
    int width, height;
    width = w ? w : c->w;
    height = h ? h : c->h;

    // halve frames if enough events are queried
    frames = frames / 1 + (XEventsQueued(dpy, QueuedAlready) > 50);

    // No animation if even more events are queried
    if (!frames || XEventsQueued(dpy, QueuedAlready) > 100) {
        if (resetpos)
            resize(c, c->x, c->y, width, height, 0);
        else
            resize(c, x, y, width, height, 1);
        return;
    }

    int time;
    int oldx, oldy;

    // prevent oversizing when minimizing/unminimizing
    if (width > c->mon->mw - (2 * c->border_width))
        width = c->mon->ww - (2 * c->border_width);

    if (height > c->mon->wh - (2 * c->border_width))
        height = c->mon->wh - (2 * c->border_width);

    time = 1;
    oldx = c->x;
    oldy = c->y;

    if (animated && (abs(oldx - x) > 10 || abs(oldy - y) > 10 ||
                     abs(w - c->w) > 10 || abs(h - c->h) > 10)) {
        if (x == c->x && y == c->y && c->w < selmon->mw - 50) {
            animateclient(c, c->x + (width - c->w), c->y + (height - c->h), 0,
                          0, frames, 0);
        } else {
            while (time < frames) {
                resize(
                    c,
                    oldx + easeOutCubic(((double)time / frames)) * (x - oldx),
                    oldy + easeOutCubic(((double)time / frames)) * (y - oldy),
                    width, height, 1);
                /* ⚡ Bolt: Flush requests to ensure smooth animation without blocking */
                XFlush(dpy);
                time++;
                usleep(15000);
            }
        }
    }

    if (resetpos)
        resize(c, oldx, oldy, width, height, 0);
    else
        resize(c, x, y, width, height, 1);
}

static void animscroll(const Arg *arg, int dir) {
    if (&overviewlayout == tiling_layout_func(selmon)) {
        direction_focus(&((Arg){.ui = dir == DirRight ? 1 : 3}));
        return;
    }

    Client *tempc;
    int modifier = (dir == DirRight) ? 1 : -1;

    // windows like behaviour in floating layout
    if (selmon->sel && NULL == tiling_layout_func(selmon)) {
        XSetWindowBorder(dpy, selmon->sel->win,
                         borderscheme[SchemeBorderTileFocus].pixel);
        changesnap(selmon->sel, dir == DirRight ? 1 : 3);
        return;
    }

    if (selmon->pertag->current_tag == 0)
        return;

    if (dir == DirLeft && selmon->pertag->current_tag == 1)
        return;

    if (dir == DirRight && selmon->pertag->current_tag >= 20)
        return;

    if (animated) {
        int tmpcounter = 0;
        int target = selmon->pertag->current_tag + modifier;
        for (tempc = selmon->clients; tempc; tempc = tempc->next) {
            if (tempc->tags & 1 << (target - 1) && !tempc->isfloating &&
                selmon->pertag &&
                selmon->pertag->ltidxs[target][0]->arrange != NULL) {
                if (!tmpcounter) {
                    tmpcounter = 1;
                    tempc->x = tempc->x + (modifier * 200);
                }
            }
        }
    }

    if (dir == DirRight)
        viewtoright(arg);
    else
        viewtoleft(arg);
}

void animleft(const Arg *arg) { animscroll(arg, DirLeft); }

void animright(const Arg *arg) { animscroll(arg, DirRight); }

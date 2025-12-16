/* See LICENSE file for copyright and license details. */

#include <unistd.h>

#include "animation.h"

/* External declarations for variables defined in instantwm.c */
extern Display *dpy;
extern Monitor *selmon;
extern int animated;

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
    if (width > c->mon->mw - (2 * c->bw))
        width = c->mon->ww - (2 * c->bw);

    if (height > c->mon->wh - (2 * c->bw))
        height = c->mon->wh - (2 * c->bw);

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

void animleft(const Arg *arg) {

    if (&overviewlayout == selmon->lt[selmon->sellt]->arrange) {
        directionfocus(&((Arg){.ui = 3}));
        return;
    }

    Client *tempc;

    // windows like behaviour in floating layout
    if (selmon->sel && NULL == selmon->lt[selmon->sellt]->arrange) {
        XSetWindowBorder(dpy, selmon->sel->win,
                         borderscheme[SchemeBorderTileFocus].pixel);
        changesnap(selmon->sel, 3);
        return;
    }

    if (selmon->pertag->curtag == 1 || selmon->pertag->curtag == 0)
        return;

    if (animated) {
        int tmpcounter = 0;
        for (tempc = selmon->clients; tempc; tempc = tempc->next) {
            if (tempc->tags & 1 << (selmon->pertag->curtag - 2) &&
                !tempc->isfloating && selmon->pertag &&
                selmon->pertag->ltidxs[selmon->pertag->curtag - 1][0]
                        ->arrange != NULL) {
                if (!tmpcounter) {
                    tmpcounter = 1;
                    tempc->x = tempc->x - 200;
                }
            }
        }
    }

    viewtoleft(arg);
}

void animright(const Arg *arg) {

    Client *tempc;
    int tmpcounter = 0;

    if (&overviewlayout == selmon->lt[selmon->sellt]->arrange) {
        directionfocus(&((Arg){.ui = 1}));
        return;
    }

    // snap window to the right
    if (selmon->sel && NULL == selmon->lt[selmon->sellt]->arrange) {
        XSetWindowBorder(dpy, selmon->sel->win,
                         borderscheme[SchemeBorderTileFocus].pixel);
        changesnap(selmon->sel, 1);
        return;
    }

    if (selmon->pertag->curtag >= 20 || selmon->pertag->curtag == 0)
        return;

    if (animated) {
        for (tempc = selmon->clients; tempc; tempc = tempc->next) {
            if (tempc->tags & 1 << selmon->pertag->curtag &&
                !tempc->isfloating && selmon->pertag &&
                selmon->pertag->ltidxs[selmon->pertag->curtag + 1][0]
                        ->arrange != NULL) {
                if (!tmpcounter) {
                    tmpcounter = 1;
                    tempc->x = tempc->x + 200;
                }
            }
        }
    }

    viewtoright(arg);
}

/* See LICENSE file for copyright and license details. */

#include "floating.h"
#include "animation.h"
#include "push.h"

/* External declarations for variables defined in instantwm.c */
extern Display *dpy;
extern Monitor *selmon;
extern Monitor *mons;
extern int bh;
extern int animated;
extern Clr *borderscheme;

void savefloating(Client *c) {
    c->sfx = c->x;
    c->sfy = c->y;
    c->sfw = c->w;
    c->sfh = c->h;
}

void restorefloating(Client *c) {
    c->x = c->sfx;
    c->y = c->sfy;
    c->w = c->sfw;
    c->h = c->sfh;
}

void savebw(Client *c) {
    if (!c->bw || c->bw == 0)
        return;
    c->oldbw = c->bw;
}

void restorebw(Client *c) {
    if (!c->oldbw || c->oldbw == 0)
        return;
    c->bw = c->oldbw;
}

void applysize(Client *c) { resize(c, c->x + 1, c->y, c->w, c->h, 0); }

int checkfloating(Client *c) {
    if (c->isfloating || NULL == selmon->lt[selmon->sellt]->arrange)
        return 1;
    return 0;
}

int visible(Client *c) {
    Monitor *m;
    if (!c)
        return 0;
    for (m = mons; m; m = m->next) {
        if (c->tags & m->seltags && c->mon == m)
            return 1;
    }
    return 0;
}

void saveallfloating(Monitor *m) {
    int i;
    Client *c;
    for (i = 1; i < 20; ++i) {
        if (m->pertag->ltidxs[i][m->pertag->sellts[i]]->arrange != NULL)
            continue;
        for (c = m->clients; c; c = c->next) {
            if (c->tags & (1 << (i - 1)) && c->snapstatus == 0)
                savefloating(c);
        }
    }
}

void restoreallfloating(Monitor *m) {
    int i;
    Client *c;
    for (i = 1; i < 20; ++i) {
        if (m->pertag->ltidxs[i][m->pertag->sellts[i]]->arrange != NULL)
            continue;
        for (c = m->clients; c; c = c->next) {
            if (c->tags & (1 << (i - 1)) && c->snapstatus == 0)
                restorefloating(c);
        }
    }
}

void resetsnap(Client *c) {
    if (!c->snapstatus)
        return;
    if (c->isfloating || NULL == selmon->lt[selmon->sellt]->arrange) {
        c->snapstatus = 0;
        restorebw(c);
        restorefloating(c);
        applysize(c);
    }
}

void applysnap(Client *c, Monitor *m) {
    int mony = m->my + (bh * m->showbar);
    if (c->snapstatus != 9)
        restorebw(c);
    switch (c->snapstatus) {
    case 0:
        checkanimate(c, c->sfx, c->sfy, c->sfw, c->sfh, 7, 0);
        break;
    case 1:
        checkanimate(c, m->mx, mony, m->mw, m->mh / 2, 7, 0);
        break;
    case 2:
        checkanimate(c, m->mx + m->mw / 2, mony, m->mw / 2, m->mh / 2, 7, 0);
        break;
    case 3:
        checkanimate(c, m->mx + m->mw / 2, mony, m->mw / 2 - c->bw * 2,
                     m->wh - c->bw * 2, 7, 0);
        break;
    case 4:
        checkanimate(c, m->mx + m->mw / 2, mony + m->mh / 2, m->mw / 2,
                     m->wh / 2, 7, 0);
        break;
    case 5:
        checkanimate(c, m->mx, mony + m->mh / 2, m->mw, m->mh / 2, 7, 0);
        break;
    case 6:
        checkanimate(c, m->mx, mony + m->mh / 2, m->mw / 2, m->wh / 2, 7, 0);
        break;
    case 7:
        checkanimate(c, m->mx, mony, m->mw / 2, m->wh, 7, 0);
        break;
    case 8:
        checkanimate(c, m->mx, mony, m->mw / 2, m->mh / 2, 7, 0);
        break;
    case 9:
        savebw(c);
        c->bw = 0;
        checkanimate(c, m->mx, mony, m->mw - c->bw * 2, m->mh + c->bw * 2, 7,
                     0);
        if (c == selmon->sel)
            XRaiseWindow(dpy, c->win);
        break;
    default:
        break;
    }
}

void changesnap(Client *c, int snapmode) {
    int snapmatrix[10][4] = {
        {9, 3, 5, 7}, // normal
        {9, 2, 0, 8}, // top half
        {2, 2, 3, 1}, // top right
        {2, 3, 4, 0}, // right half
        {3, 4, 4, 5}, // bottom right
        {0, 4, 5, 6}, // bottom half
        {7, 5, 6, 6}, // bottom left
        {8, 0, 6, 7}, // left half
        {8, 1, 7, 1}, // top left
        {1, 3, 0, 7}, // maximized
    };
    int tempsnap;
    if (!c->snapstatus)
        c->snapstatus = 0;
    if (c->snapstatus == 0 && checkfloating(c))
        savefloating(c);
    tempsnap = c->snapstatus;
    c->snapstatus = snapmatrix[tempsnap][snapmode];
    applysnap(c, c->mon);
    warp(c);
    focus(c);
}

void tempfullscreen(const Arg *arg) {
    if (selmon->fullscreen) {
        Client *c;
        c = selmon->fullscreen;
        if (c->isfloating || NULL == selmon->lt[selmon->sellt]->arrange) {
            restorefloating(c);
            applysize(c);
        }
        selmon->fullscreen = NULL;
    } else {
        if (!selmon->sel)
            return;
        selmon->fullscreen = selmon->sel;
        if (selmon->sel->isfloating ||
            NULL == selmon->lt[selmon->sellt]->arrange)
            savefloating(selmon->fullscreen);
    }

    if (animated) {
        animated = 0;
        arrange(selmon);
        animated = 1;
    } else {
        arrange(selmon);
    }
    if (selmon->fullscreen)
        XRaiseWindow(dpy, selmon->fullscreen->win);
}

void togglefloating(const Arg *arg) {
    if (!selmon->sel || selmon->sel == selmon->overlay)
        return;
    if (selmon->sel->isfullscreen &&
        !selmon->sel->isfakefullscreen) /* no support for fullscreen windows */
        return;
    selmon->sel->isfloating = !selmon->sel->isfloating || selmon->sel->isfixed;
    if (selmon->sel->isfloating) {
        // make window float
        restorebw(selmon->sel);
        XSetWindowBorder(dpy, selmon->sel->win,
                         borderscheme[SchemeBorderFloatFocus].pixel);
        animateclient(selmon->sel, selmon->sel->sfx, selmon->sel->sfy,
                      selmon->sel->sfw, selmon->sel->sfh, 7, 0);
    } else {
        // make window tile
        selmon->clientcount = clientcount();
        if (selmon->clientcount <= 1 && !selmon->sel->snapstatus) {
            savebw(selmon->sel);
            selmon->sel->bw = 0;
        }
        XSetWindowBorder(dpy, selmon->sel->win,
                         borderscheme[SchemeBorderTileFocus].pixel);
        /* save last known float dimensions */
        selmon->sel->sfx = selmon->sel->x;
        selmon->sel->sfy = selmon->sel->y;
        selmon->sel->sfw = selmon->sel->w;
        selmon->sel->sfh = selmon->sel->h;
    }
    arrange(selmon);
}

void changefloating(Client *c) {
    if (!c)
        return;
    if (c->isfullscreen &&
        !c->isfakefullscreen) /* no support for fullscreen windows */
        return;
    c->isfloating = !c->isfloating || c->isfixed;
    if (c->isfloating)
        /* restore last known float dimensions */
        resize(c, c->sfx, c->sfy, c->sfw, c->sfh, False);
    else {
        /* save last known float dimensions */
        c->sfx = c->x;
        c->sfy = c->y;
        c->sfw = c->w;
        c->sfh = c->h;
    }
    arrange(selmon);
}

void centerwindow(const Arg *arg) {
    if (!selmon->sel || selmon->sel == selmon->overlay)
        return;
    Client *c;
    c = selmon->sel;
    if (selmon->lt[selmon->sellt]->arrange && !c->isfloating)
        return;

    int w, h, mw, mh;
    w = c->w;
    h = c->h;
    mw = selmon->ww;
    mh = selmon->wh;
    if (w > mw || h > mh)
        return;
    if (selmon->showbar)
        resize(c, selmon->mx + (mw / 2) - (w / 2),
               selmon->my + (mh / 2) - (h / 2) + bh, c->w, c->h, True);
    else
        resize(c, selmon->mx + (mw / 2) - (w / 2),
               selmon->my + (mh / 2) - (h / 2) - bh, c->w, c->h, True);
}

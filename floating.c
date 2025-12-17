/* See LICENSE file for copyright and license details. */

#include "floating.h"
#include "animation.h"
#include "focus.h"
#include "push.h"

/* External declarations for variables defined in instantwm.c */
extern Display *dpy;
extern Monitor *selmon;
extern Monitor *mons;
extern int bh;
extern int animated;
extern Clr *borderscheme;
extern int numtags;

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
    for (i = 1; i < numtags && i < MAX_TAGS; ++i) {
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
    for (i = 1; i < numtags && i < MAX_TAGS; ++i) {
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
    if (c->snapstatus != SnapMaximized)
        restorebw(c);
    switch (c->snapstatus) {
    case SnapNone:
        checkanimate(c, c->sfx, c->sfy, c->sfw, c->sfh, 7, 0);
        break;
    case SnapTop:
        checkanimate(c, m->mx, mony, m->mw, m->mh / 2, 7, 0);
        break;
    case SnapTopRight:
        checkanimate(c, m->mx + m->mw / 2, mony, m->mw / 2, m->mh / 2, 7, 0);
        break;
    case SnapRight:
        checkanimate(c, m->mx + m->mw / 2, mony, m->mw / 2 - c->bw * 2,
                     m->wh - c->bw * 2, 7, 0);
        break;
    case SnapBottomRight:
        checkanimate(c, m->mx + m->mw / 2, mony + m->mh / 2, m->mw / 2,
                     m->wh / 2, 7, 0);
        break;
    case SnapBottom:
        checkanimate(c, m->mx, mony + m->mh / 2, m->mw, m->mh / 2, 7, 0);
        break;
    case SnapBottomLeft:
        checkanimate(c, m->mx, mony + m->mh / 2, m->mw / 2, m->wh / 2, 7, 0);
        break;
    case SnapLeft:
        checkanimate(c, m->mx, mony, m->mw / 2, m->wh, 7, 0);
        break;
    case SnapTopLeft:
        checkanimate(c, m->mx, mony, m->mw / 2, m->mh / 2, 7, 0);
        break;
    case SnapMaximized:
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

/* Set a client to floating state (does nothing if already floating) */
void setfloating(Client *c, int should_arrange) {
    if (!c)
        return;
    if (c->isfullscreen && !c->isfakefullscreen)
        return;
    if (c->isfloating)
        return; /* already floating */

    c->isfloating = 1;
    /* restore last known float dimensions */
    resize(c, c->sfx, c->sfy, c->sfw, c->sfh, False);

    if (should_arrange)
        arrange(selmon);
}

/* Set a client to tiled state (does nothing if already tiled) */
void settiled(Client *c, int should_arrange) {
    if (!c)
        return;
    if (c->isfullscreen && !c->isfakefullscreen)
        return;
    if (!c->isfloating && !c->isfixed)
        return; /* already tiled */

    c->isfloating = 0;
    /* save last known float dimensions */
    c->sfx = c->x;
    c->sfy = c->y;
    c->sfw = c->w;
    c->sfh = c->h;

    if (should_arrange)
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

// move a client with the mouse and keyboard
void moveresize(const Arg *arg) {
    /* only floating windows can be moved */
    Client *c;
    c = selmon->sel;

    if (selmon->lt[selmon->sellt]->arrange && !c->isfloating)
        return;

    int mstrength = 40;
    int mpositions[4][2] = {{0, mstrength},
                            {0, (-1) * mstrength},
                            {mstrength, 0},
                            {(-1) * mstrength, 0}};
    int nx = (c->x + mpositions[arg->i][0]);
    int ny = (c->y + mpositions[arg->i][1]);

    if (nx < selmon->mx)
        nx = selmon->mx;
    if (ny < selmon->my)
        ny = selmon->my;

    if ((ny + c->h) > (selmon->my + selmon->mh))
        ny = ((selmon->mh + selmon->my) - c->h - c->bw * 2);

    if ((nx + c->w) > (selmon->mx + selmon->mw))
        nx = ((selmon->mw + selmon->mx) - c->w - c->bw * 2);

    animateclient(c, nx, ny, c->w, c->h, 5, 0);
    warp(c);
}

void keyresize(const Arg *arg) {

    if (!selmon->sel)
        return;

    Client *c;
    c = selmon->sel;

    int mstrength = 40;
    int mpositions[4][2] = {{0, mstrength},
                            {0, (-1) * mstrength},
                            {mstrength, 0},
                            {(-1) * mstrength, 0}};

    int nw = (c->w + mpositions[arg->i][0]);
    int nh = (c->h + mpositions[arg->i][1]);

    resetsnap(c);

    if (selmon->lt[selmon->sellt]->arrange && !c->isfloating)
        return;

    warp(c);

    resize(c, c->x, c->y, nw, nh, True);
}

void upscaleclient(const Arg *arg) {
    Client *c;

    if (!arg->v) {
        if (selmon->sel)
            c = selmon->sel;
        else
            return;
    } else {
        c = (Client *)arg->v;
    }

    scaleclient(c, 30);
}

void downscaleclient(const Arg *arg) {
    Client *c;

    if (!arg->v) {
        if (selmon->sel)
            c = selmon->sel;
        else
            return;
    } else {
        c = (Client *)arg->v;
    }

    if (!c->isfloating) {
        focus(c);
        togglefloating(NULL);
    }

    scaleclient(c, -30);
}

void scaleclient(Client *c, int scale) {
    int x, y, w, h;
    if (!c->isfloating)
        return;

    w = c->w + scale;
    h = c->h + scale;
    x = c->x - (scale / 2);
    y = c->y - (scale / 2);

    if ((double)c->h / c->w < 0.25 || (double)c->h / c->w < 0.25) {
        h = c->h + scale;
        h = c->x - scale / 2;
        h = c->y - scale / 2;
    }

    if (x < selmon->mx)
        x = selmon->mx;

    if (w > selmon->mw)
        w = selmon->mw;

    if (h > selmon->mh)
        h = selmon->mh;
    if ((h + y) > selmon->my + selmon->mh)
        y = selmon->mh - h;

    if (y < bh)
        y = bh;
    animateclient(c, x, y, w, h, 3, 0);
}

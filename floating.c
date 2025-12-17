/* See LICENSE file for copyright and license details. */

#include "floating.h"
#include "animation.h"
#include "focus.h"
#include "layouts.h"
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
    c->saved_float_x = c->x;
    c->saved_float_y = c->y;
    c->saved_float_width = c->w;
    c->saved_float_height = c->h;
}

void restorefloating(Client *c) {
    c->x = c->saved_float_x;
    c->y = c->saved_float_y;
    c->w = c->saved_float_width;
    c->h = c->saved_float_height;
}

void savebw(Client *c) {
    if (!c->border_width || c->border_width == 0)
        return;
    c->old_border_width = c->border_width;
}

/* Restore the client's saved border width (bw = border_width) */
void restore_border_width(Client *c) {
    if (!c->old_border_width || c->old_border_width == 0)
        return;
    c->border_width = c->old_border_width;
}

void applysize(Client *c) { resize(c, c->x + 1, c->y, c->w, c->h, 0); }

int checkfloating(Client *c) {
    if (c->isfloating || NULL == tiling_layout_func(selmon))
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
    if (c->isfloating || NULL == tiling_layout_func(selmon)) {
        c->snapstatus = 0;
        restore_border_width(c);
        restorefloating(c);
        applysize(c);
    }
}

void applysnap(Client *c, Monitor *m) {
    int mony = m->my + (bh * m->showbar);
    if (c->snapstatus != SnapMaximized)
        restore_border_width(c);
    switch (c->snapstatus) {
    case SnapNone:
        checkanimate(c, c->saved_float_x, c->saved_float_y,
                     c->saved_float_width, c->saved_float_height, 7, 0);
        break;
    case SnapTop:
        checkanimate(c, m->mx, mony, m->mw, m->mh / 2, 7, 0);
        break;
    case SnapTopRight:
        checkanimate(c, m->mx + m->mw / 2, mony, m->mw / 2, m->mh / 2, 7, 0);
        break;
    case SnapRight:
        checkanimate(c, m->mx + m->mw / 2, mony,
                     m->mw / 2 - c->border_width * 2,
                     m->wh - c->border_width * 2, 7, 0);
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
        c->border_width = 0;
        checkanimate(c, m->mx, mony, m->mw - c->border_width * 2,
                     m->mh + c->border_width * 2, 7, 0);
        if (c == selmon->sel)
            XRaiseWindow(dpy, c->win);
        break;
    default:
        break;
    }
}

/* Snap navigation matrix: [current_snap][direction] -> new_snap
 * Direction indices: 0=Up, 1=Right, 2=Down, 3=Left
 * Each row comment shows: [from_snap] -> up/right/down/left */
/* clang-format off */
static const int snapmatrix[10][4] = {
    {SnapMaximized, SnapRight, SnapBottom, SnapLeft},           // SnapNone
    {SnapMaximized, SnapTopRight, SnapNone, SnapTopLeft},       // SnapTop
    {SnapTopRight, SnapTopRight, SnapRight, SnapTop},           // SnapTopRight
    {SnapTopRight, SnapRight, SnapBottomRight, SnapNone},       // SnapRight
    {SnapRight, SnapBottomRight, SnapBottomRight, SnapBottom},  // SnapBottomRight
    {SnapNone, SnapBottomRight, SnapBottom, SnapBottomLeft},    // SnapBottom
    {SnapLeft, SnapBottom, SnapBottomLeft, SnapBottomLeft},     // SnapBottomLeft
    {SnapTopLeft, SnapNone, SnapBottomLeft, SnapLeft},          // SnapLeft
    {SnapTopLeft, SnapTop, SnapLeft, SnapTop},                  // SnapTopLeft
    {SnapTop, SnapRight, SnapNone, SnapLeft},                   // SnapMaximized
};
/* clang-format on */

void changesnap(Client *c, int snapmode) {
    int tempsnap;
    if (!c->snapstatus)
        c->snapstatus = 0;
    if (c->snapstatus == 0 && checkfloating(c))
        savefloating(c);
    tempsnap = c->snapstatus;
    c->snapstatus = snapmatrix[tempsnap][snapmode];
    applysnap(c, c->mon);
    warp_cursor_to_client(c);
    focus(c);
}

void temp_fullscreen(const Arg *arg) {
    if (selmon->fullscreen) {
        Client *c;
        c = selmon->fullscreen;
        if (c->isfloating || NULL == tiling_layout_func(selmon)) {
            restorefloating(c);
            applysize(c);
        }
        selmon->fullscreen = NULL;
    } else {
        if (!selmon->sel)
            return;
        selmon->fullscreen = selmon->sel;
        if (selmon->sel->isfloating || NULL == tiling_layout_func(selmon))
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

void toggle_floating(const Arg *arg) {
    if (!selmon->sel || selmon->sel == selmon->overlay)
        return;
    if (selmon->sel->is_fullscreen &&
        !selmon->sel->isfakefullscreen) /* no support for fullscreen windows */
        return;
    selmon->sel->isfloating = !selmon->sel->isfloating || selmon->sel->isfixed;
    if (selmon->sel->isfloating) {
        // make window float
        restore_border_width(selmon->sel);
        XSetWindowBorder(dpy, selmon->sel->win,
                         borderscheme[SchemeBorderFloatFocus].pixel);
        animateclient(selmon->sel, selmon->sel->saved_float_x,
                      selmon->sel->saved_float_y,
                      selmon->sel->saved_float_width,
                      selmon->sel->saved_float_height, 7, 0);
    } else {
        // make window tile
        selmon->clientcount = clientcount();
        if (selmon->clientcount <= 1 && !selmon->sel->snapstatus) {
            savebw(selmon->sel);
            selmon->sel->border_width = 0;
        }
        XSetWindowBorder(dpy, selmon->sel->win,
                         borderscheme[SchemeBorderTileFocus].pixel);
        /* save last known float dimensions */
        selmon->sel->saved_float_x = selmon->sel->x;
        selmon->sel->saved_float_y = selmon->sel->y;
        selmon->sel->saved_float_width = selmon->sel->w;
        selmon->sel->saved_float_height = selmon->sel->h;
    }
    arrange(selmon);
}

/* Toggle floating state for any client (internal use).
 * Unlike toggle_floating, this:
 * - Takes a client parameter instead of using selmon->sel
 * - Does not animate the transition (uses resize instead of animateclient)
 * - Does not update border colors
 * - Does not handle clientcount or border width for single-window layouts
 * Used primarily for overlay windows where visual effects aren't needed. */
void changefloating(Client *c) {
    if (!c)
        return;
    if (c->is_fullscreen &&
        !c->isfakefullscreen) /* no support for fullscreen windows */
        return;
    c->isfloating = !c->isfloating || c->isfixed;
    if (c->isfloating)
        /* restore last known float dimensions */
        resize(c, c->saved_float_x, c->saved_float_y, c->saved_float_width,
               c->saved_float_height, False);
    else {
        /* save last known float dimensions */
        c->saved_float_x = c->x;
        c->saved_float_y = c->y;
        c->saved_float_width = c->w;
        c->saved_float_height = c->h;
    }
    arrange(selmon);
}

/* Set a client to floating state (does nothing if already floating) */
void setfloating(Client *c, int should_arrange) {
    if (!c)
        return;
    if (c->is_fullscreen && !c->isfakefullscreen)
        return;
    if (c->isfloating)
        return; /* already floating */

    c->isfloating = 1;
    /* restore last known float dimensions */
    resize(c, c->saved_float_x, c->saved_float_y, c->saved_float_width,
           c->saved_float_height, False);

    if (should_arrange)
        arrange(selmon);
}

/* Set a client to tiled state (does nothing if already tiled) */
void set_tiled(Client *c, int should_arrange) {
    if (!c)
        return;
    if (c->is_fullscreen && !c->isfakefullscreen)
        return;
    if (!c->isfloating && !c->isfixed)
        return; /* already tiled */

    c->isfloating = 0;
    /* save last known float dimensions */
    c->saved_float_x = c->x;
    c->saved_float_y = c->y;
    c->saved_float_width = c->w;
    c->saved_float_height = c->h;

    if (should_arrange)
        arrange(selmon);
}

void center_window(const Arg *arg) {
    if (!selmon->sel || selmon->sel == selmon->overlay)
        return;
    Client *c;
    c = selmon->sel;
    if (tiling_layout_func(selmon) && !c->isfloating)
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

/* Move a floating client using keyboard (direction indexed by arg->i).
 * Direction indices: 0=Down, 1=Up, 2=Right, 3=Left */
void moveresize(const Arg *arg) {
    /* only floating windows can be moved */
    Client *c;
    c = selmon->sel;

    if (tiling_layout_func(selmon) && !c->isfloating)
        return;

    int move_step = 40;
    /* Delta offsets for each direction: [direction][x_delta, y_delta] */
    int move_deltas[4][2] = {
        {0, move_step}, {0, -move_step}, {move_step, 0}, {-move_step, 0}};
    int nx = (c->x + move_deltas[arg->i][0]);
    int ny = (c->y + move_deltas[arg->i][1]);

    if (nx < selmon->mx)
        nx = selmon->mx;
    if (ny < selmon->my)
        ny = selmon->my;

    if ((ny + c->h) > (selmon->my + selmon->mh))
        ny = ((selmon->mh + selmon->my) - c->h - c->border_width * 2);

    if ((nx + c->w) > (selmon->mx + selmon->mw))
        nx = ((selmon->mw + selmon->mx) - c->w - c->border_width * 2);

    animateclient(c, nx, ny, c->w, c->h, 5, 0);
    warp_cursor_to_client(c);
}

/* Resize a floating client using keyboard (direction indexed by arg->i).
 * Direction indices: 0=TallerDown, 1=ShorterUp, 2=WiderRight, 3=NarrowerLeft */
void keyresize(const Arg *arg) {
    if (!selmon->sel)
        return;

    Client *c;
    c = selmon->sel;

    int resize_step = 40;
    /* Size deltas for each direction: [direction][width_delta, height_delta] */
    int resize_deltas[4][2] = {{0, resize_step},
                               {0, -resize_step},
                               {resize_step, 0},
                               {-resize_step, 0}};

    int nw = (c->w + resize_deltas[arg->i][0]);
    int nh = (c->h + resize_deltas[arg->i][1]);

    resetsnap(c);

    if (tiling_layout_func(selmon) && !c->isfloating)
        return;

    warp_cursor_to_client(c);

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
        toggle_floating(NULL);
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

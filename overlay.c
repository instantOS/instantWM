/* See LICENSE file for copyright and license details. */

#include "overlay.h"
#include "animation.h"
#include "floating.h"

/* External declarations for variables defined in instantwm.c */
extern Display *dpy;
extern Monitor *selmon;
extern Monitor *mons;
extern int bh;

int overlayexists() {
    Client *c;
    Monitor *m;
    if (!selmon->overlay)
        return 0;

    for (m = mons; m; m = m->next) {
        for (c = m->clients; c; c = c->next) {
            if (c == m->overlay) {
                return 1;
            }
        }
    }

    return 0;
}

void createoverlay(const Arg *arg) {
    Monitor *m;
    Monitor *tm;
    if (!selmon->sel)
        return;
    if (selmon->sel == selmon->fullscreen)
        tempfullscreen(NULL);
    if (selmon->sel == selmon->overlay) {
        resetoverlay();
        for (tm = mons; tm; tm = tm->next) {
            tm->overlay = NULL;
        }
        return;
    }

    Client *tempclient = selmon->sel;

    resetoverlay();

    for (m = mons; m; m = m->next) {
        m->overlay = tempclient;
        m->overlaystatus = 0;
    }

    savebw(tempclient);
    tempclient->bw = 0;
    tempclient->islocked = 1;
    if (!selmon->overlay->isfloating) {
        changefloating(selmon->overlay);
    }

    if (selmon->overlaymode == 0 || selmon->overlaymode == 2)
        selmon->overlay->h = ((selmon->wh) / 3);
    else
        selmon->overlay->w = ((selmon->ww) / 3);

    XRaiseWindow(dpy, tempclient->win);
    showoverlay(NULL);
}

void resetoverlay() {
    if (!overlayexists())
        return;
    selmon->overlay->tags = selmon->tagset[selmon->seltags];
    selmon->overlay->bw = selmon->overlay->oldbw;
    selmon->overlay->issticky = 0;
    selmon->overlay->islocked = 0;
    changefloating(selmon->overlay);
    arrange(selmon);
    focus(selmon->overlay);
}

void showoverlay(const Arg *arg) {
    Monitor *m;
    if (!overlayexists() || selmon->overlaystatus)
        return;

    int yoffset = selmon->showbar ? bh : 0;

    Client *c;
    for (c = selmon->clients; c; c = c->next) {
        if (c->tags & (1 << (selmon->pertag->curtag - 1)) && c->isfullscreen &&
            !c->isfakefullscreen) {
            yoffset = 0;
            break;
        }
    }

    for (m = mons; m; m = m->next) {
        m->overlaystatus = 1;
    }

    c = selmon->overlay;

    detach(c);
    detachstack(c);
    c->mon = selmon;
    attach(c);
    attachstack(c);
    selmon->overlay->isfloating = 1;

    if (c->islocked) {
        switch (selmon->overlaymode) {
        case OverlayTop:
            resize(c, selmon->mx + 20, selmon->my + yoffset - c->h,
                   selmon->ww - 40, c->h, True);
            break;
        case OverlayRight:
            resize(c, selmon->mx + selmon->mw - 20, selmon->my + 40, c->w,
                   selmon->mh - 80, True);
            break;
        case OverlayBottom:
            resize(c, selmon->mx + 20, selmon->my + selmon->mh, selmon->ww - 40,
                   c->h, True);
            break;
        case OverlayLeft:
            resize(c, selmon->mx - c->w + 20, selmon->my + 40, c->w,
                   selmon->mh - 80, True);
            break;
        default:
            selmon->overlaymode = OverlayTop;
            break;
        }
    }

    c->tags = selmon->tagset[selmon->seltags];

    if (!c->isfloating) {
        changefloating(selmon->overlay);
    }

    if (c->islocked) {
        XRaiseWindow(dpy, c->win);
        switch (selmon->overlaymode) {
        case OverlayTop:
            animateclient(c, c->x, selmon->my + yoffset, 0, 0, 15, 0);
            break;
        case OverlayRight:
            animateclient(c, selmon->mx + selmon->mw - c->w, selmon->my + 40, 0,
                          0, 15, 0);
            break;
        case OverlayBottom:
            animateclient(c, selmon->mx + 20, selmon->my + selmon->mh - c->h, 0,
                          0, 15, 0);
            break;
        case OverlayLeft:
            animateclient(c, selmon->mx, selmon->my + 40, 0, 0, 15, 0);
            break;
        default:
            selmon->overlaymode = OverlayTop;
            break;
        }
        c->issticky = 1;
    }

    c->bw = 0;
    /* arrange(selmon); */
    focus(c);
    XRaiseWindow(dpy, c->win);
}

void hideoverlay(const Arg *arg) {
    if (!overlayexists() || !selmon->overlaystatus)
        return;

    Client *c;
    Monitor *m;
    c = selmon->overlay;
    c->issticky = 0;
    if (c == selmon->fullscreen)
        tempfullscreen(NULL);
    if (c->islocked) {
        switch (selmon->overlaymode) {
        case OverlayTop:
            animateclient(c, c->x, 0 - c->h, 0, 0, 15, 0);
            break;
        case OverlayRight:
            animateclient(c, selmon->mx + selmon->mw, selmon->mx + selmon->mw,
                          0, 0, 15, 0);
            break;
        case OverlayBottom:
            animateclient(c, c->x, selmon->mh + selmon->my, 0, 0, 15, 0);
            break;
        case OverlayLeft:
            animateclient(c, selmon->mx - c->w, 40, 0, 0, 15, 0);
            break;
        default:
            selmon->overlaymode = OverlayTop;
            break;
        }
    }

    for (m = mons; m; m = m->next) {
        m->overlaystatus = 0;
    }

    selmon->overlay->tags = 0;
    focus(NULL);
    arrange(selmon);
}

void setoverlay(const Arg *arg) {

    if (!overlayexists()) {
        return;
    }

    if (!selmon->overlaystatus) {
        showoverlay(NULL);
    } else {
        if (ISVISIBLE(selmon->overlay)) {
            hideoverlay(NULL);
        } else {
            showoverlay(NULL);
        }
    }
}

void setoverlaymode(int mode) {
    Monitor *m;
    for (m = mons; m; m = m->next) {
        m->overlaymode = mode;
    }

    if (!selmon->overlay)
        return;

    if (mode == OverlayTop || mode == OverlayBottom)
        selmon->overlay->h = selmon->wh / 3;
    else
        selmon->overlay->w = selmon->ww / 3;

    if (selmon->overlaystatus) {
        hideoverlay(NULL);
        showoverlay(NULL);
    }
}

void resetoverlaysize() {
    if (!selmon->overlay)
        return;
    Client *c;
    c = selmon->overlay;
    selmon->overlay->isfloating = 1;
    switch (selmon->overlaymode) {
    case OverlayTop:
        resize(c, selmon->mx + 20, bh, selmon->ww - 40, (selmon->wh) / 3, True);
        break;
    case OverlayRight:
        resize(c, selmon->mx + selmon->mw - c->w, 40, selmon->mw / 3,
               selmon->mh - 80, True);
        break;
    case OverlayBottom:
        resize(c, selmon->mx + 20, selmon->my + selmon->mh - c->h,
               selmon->ww - 40, (selmon->wh) / 3, True);
        break;
    case OverlayLeft:
        resize(c, selmon->mx, 40, selmon->mw / 3, selmon->mh - 80, True);
        break;
    default:
        selmon->overlaymode = OverlayTop;
        break;
    }
}

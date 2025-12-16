/* See LICENSE file for copyright and license details. */

#include "tags.h"
#include "animation.h"
#include "bar.h"
#include "floating.h"
#include "instantwm.h"
#include "layouts.h"
#include "overlay.h"
#include "scratchpad.h"
#include "systray.h"
#include "toggles.h"
#include "util.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define MAX_TAGLEN 16

/* variables from instantwm.c and config.h */
extern Monitor *selmon;
extern Monitor *mons;
extern Display *dpy;
extern int bh;
extern int lrpad;
extern int animated;
extern Drw *drw;
extern int tagwidth;
extern int tagprefix;
extern const unsigned int startmenusize;
extern char tags[][MAX_TAGLEN];
extern const char *tags_default[];
extern const Layout layouts[];
extern const char *tagsalt[];

extern unsigned int tagmask;
extern int numtags;

/* functions */
int computeprefix(const Arg *arg) {
    if (tagprefix && arg->ui) {
        tagprefix = 0;
        return arg->ui << 10;
    } else {
        return arg->ui;
    }
}

void nametag(const Arg *arg) {
    int i;

    char *name = (char *)arg->v;

    if (strlen(name) >= MAX_TAGLEN)
        return;

    for (i = 0; i < numtags; i++) {
        if (selmon->tagset[selmon->seltags] & (1 << i)) {
            if (strlen(name) > 0)
                strcpy(tags[i], name);
            else
                strcpy(tags[i], tags_default[i]);
        }
    }
    tagwidth = gettagwidth();
    drawbars();
}

void resetnametag(const Arg *arg) {
    for (int i = 0; i < 21; i++)
        strcpy((char *)&tags[i], tags_default[i]);
    tagwidth = gettagwidth();
    drawbars();
}

int gettagwidth() {
    int x = 0, i = 0, occ = 0;
    Client *c;

    for (c = selmon->clients; c; c = c->next)
        occ |= c->tags == 255 ? 0 : c->tags;

    do {
        // do not reserve space for vacant tags
        if (i >= 9)
            continue;
        if (selmon->showtags) {
            if (!(occ & 1 << i || selmon->tagset[selmon->seltags] & 1 << i))
                continue;
        }
        x += TEXTW(tags[i]);
    } while (++i < numtags);
    return x + startmenusize;
}

// return tag for indicator of given x coordinate
int getxtag(int ix) {
    int x, i, occ;
    Client *c;
    i = 0;
    occ = 0;
    x = startmenusize;
    for (c = selmon->clients; c; c = c->next)
        occ |= c->tags == 255 ? 0 : c->tags;

    do {
        // do not reserve space for vacant tags
        if (i >= 9)
            continue;
        if (selmon->showtags) {
            if (!(occ & 1 << i || selmon->tagset[selmon->seltags] & 1 << i))
                continue;
        }

        x += TEXTW(tags[i]);
        if (x >= ix) {
            return i;
        }
    } while (++i < numtags);
    return -1;
}

void tag(const Arg *arg) {
    int ui = computeprefix(arg);
    Client *c;
    if (selmon->sel && ui & tagmask) {
        if (selmon->sel->tags == 1 << 20)
            selmon->sel->issticky = 0;
        c = selmon->sel;
        selmon->sel->tags = ui & tagmask;
        setclienttagprop(c);
        focus(NULL);
        arrange(selmon);
    }
}

void tagall(const Arg *arg) {
    Client *c;
    int ui = computeprefix(arg);
    if (selmon->pertag->curtag == 0)
        return;
    if (selmon->sel && ui & tagmask) {
        for (c = selmon->clients; c; c = c->next) {
            if (!(c->tags & 1 << (selmon->pertag->curtag - 1)))
                continue;
            if (c->tags == 1 << 20)
                c->issticky = 0;
            c->tags = ui & tagmask;
        }
        focus(NULL);
        arrange(selmon);
    }
}

void followtag(const Arg *arg) {
    if (!selmon->sel)
        return;
    if (tagprefix) {
        tag(arg);
        tagprefix = 1;
        view(arg);

    } else {
        tag(arg);
        view(arg);
    }
}

void swaptags(const Arg *arg) {
    int ui = computeprefix(arg);
    unsigned int newtag = ui & tagmask;
    unsigned int curtag = selmon->tagset[selmon->seltags];

    if (newtag == curtag || !curtag || (curtag & (curtag - 1)))
        return;

    for (Client *c = selmon->clients; c != NULL; c = c->next) {
        if (selmon->overlay == c) {
            if (ISVISIBLE(c))
                hideoverlay(NULL);
            continue;
        }
        if ((c->tags & newtag) || (c->tags & curtag))
            c->tags ^= curtag ^ newtag;

        if (!c->tags)
            c->tags = newtag;
    }

    selmon->tagset[selmon->seltags] = newtag;

    int i, tmpnmaster, tmpsellt, tmpshowbar;
    float tmpmfact;
    const Layout *tmplt[2];
    for (i = 0; !(ui & 1 << i); i++)
        ;

    tmpnmaster = selmon->pertag->nmasters[selmon->pertag->curtag];
    tmpmfact = selmon->pertag->mfacts[selmon->pertag->curtag];
    tmpsellt = selmon->pertag->sellts[selmon->pertag->curtag];
    tmplt[selmon->sellt] =
        selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt];
    tmplt[selmon->sellt ^ 1] =
        selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt ^ 1];
    tmpshowbar = selmon->pertag->showbars[selmon->pertag->curtag];

    selmon->pertag->nmasters[selmon->pertag->curtag] =
        selmon->pertag->nmasters[i + 1];
    selmon->pertag->mfacts[selmon->pertag->curtag] =
        selmon->pertag->mfacts[i + 1];
    selmon->pertag->sellts[selmon->pertag->curtag] =
        selmon->pertag->sellts[i + 1];
    selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt] =
        selmon->pertag->ltidxs[i + 1][selmon->sellt];
    selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt ^ 1] =
        selmon->pertag->ltidxs[i + 1][selmon->sellt ^ 1];
    selmon->pertag->showbars[selmon->pertag->curtag] =
        selmon->pertag->showbars[i + 1];

    selmon->pertag->nmasters[i + 1] = tmpnmaster;
    selmon->pertag->mfacts[i + 1] = tmpmfact;
    selmon->pertag->sellts[i + 1] = tmpsellt;
    selmon->pertag->ltidxs[i + 1][selmon->sellt] = tmplt[selmon->sellt];
    selmon->pertag->ltidxs[i + 1][selmon->sellt ^ 1] = tmplt[selmon->sellt ^ 1];
    selmon->pertag->showbars[i + 1] = tmpshowbar;

    if (selmon->pertag->prevtag == i + 1)
        selmon->pertag->prevtag = selmon->pertag->curtag;
    selmon->pertag->curtag = i + 1;

    focus(NULL);
    arrange(selmon);
}

void followview(const Arg *arg) {
    if (!selmon->sel)
        return;
    Client *c = selmon->sel;
    c->tags = 1 << (selmon->pertag->prevtag - 1);
    view(&((Arg){.ui = 1 << (selmon->pertag->prevtag - 1)}));
    focus(c);
    arrange(selmon);
}

void resetsticky(Client *c) {
    if (!c->issticky)
        return;
    c->issticky = 0;
    c->tags = 1 << (selmon->pertag->curtag - 1);
}

void tagmon(const Arg *arg) {
    if (!selmon->sel || !mons->next)
        return;

    if (selmon->sel->isfloating) {
        Client *c;
        float xfact, yfact;
        c = selmon->sel;
        xfact = (float)(c->x - selmon->mx) / selmon->ww;
        yfact = (float)(c->y - selmon->my) / selmon->wh;

        sendmon(selmon->sel, dirtomon(arg->i));
        c->x = c->mon->mx + c->mon->ww * xfact;
        c->y = c->mon->my + c->mon->wh * yfact;
        arrange(c->mon);
        XRaiseWindow(dpy, c->win);
    } else {
        sendmon(selmon->sel, dirtomon(arg->i));
    }
}

void tagtoleft(const Arg *arg) {

    int oldx;
    Client *c;

    if (!selmon->sel)
        return;

    if (selmon->sel == selmon->overlay) {
        setoverlaymode(3);
        return;
    }

    if (selmon->pertag->curtag == 1)
        return;

    c = selmon->sel;
    resetsticky(c);
    oldx = c->x;
    if (!c->isfloating && animated) {
        XRaiseWindow(dpy, c->win);
        animateclient(c, c->x - (selmon->mw / 10), c->y, 0, 0, 7, 0);
    }

    int offset = 1;
    if (arg && arg->i)
        offset = arg->i;

    if (selmon->sel != NULL &&
        __builtin_popcount(selmon->tagset[selmon->seltags] & tagmask) == 1 &&
        selmon->tagset[selmon->seltags] > 1) {
        selmon->sel->tags >>= offset;
        focus(NULL);
        arrange(selmon);
    }
    c->x = oldx;
}

void tagtoright(const Arg *arg) {

    int oldx;
    Client *c;

    if (selmon->pertag->curtag == 20)
        return;

    if (!selmon->sel)
        return;

    if (selmon->sel == selmon->overlay) {
        setoverlaymode(1);
        return;
    }
    c = selmon->sel;
    resetsticky(c);
    oldx = c->x;
    if (!c->isfloating && animated) {
        XRaiseWindow(dpy, c->win);
        animateclient(c, c->x + (selmon->mw / 10), c->y, 0, 0, 7, 0);
    }

    int offset = 1;
    if (arg && arg->i)
        offset = arg->i;

    if (selmon->sel != NULL &&
        __builtin_popcount(selmon->tagset[selmon->seltags] & tagmask) == 1 &&
        selmon->tagset[selmon->seltags] & (tagmask >> 1)) {
        selmon->sel->tags <<= offset;
        focus(NULL);
        arrange(selmon);
    }
    c->x = oldx;
}

void toggletag(const Arg *arg) {
    int ui = computeprefix(arg);
    unsigned int newtags;

    if (!selmon->sel)
        return;

    if (selmon->sel->tags == 1 << 20) {
        tag(arg);
        return;
    }

    newtags = selmon->sel->tags ^ (ui & tagmask);
    if (newtags) {
        selmon->sel->tags = newtags;
        setclienttagprop(selmon->sel);
        focus(NULL);
        arrange(selmon);
    }
}

void toggleview(const Arg *arg) {
    unsigned int newtagset =
        selmon->tagset[selmon->seltags] ^ (arg->ui & tagmask);
    int i;

    if (newtagset) {
        selmon->tagset[selmon->seltags] = newtagset;

        if (newtagset == ~0) {
            selmon->pertag->prevtag = selmon->pertag->curtag;
            selmon->pertag->curtag = 0;
        }

        /* test if the user did not select the same tag */
        if (!(newtagset & 1 << (selmon->pertag->curtag - 1))) {
            selmon->pertag->prevtag = selmon->pertag->curtag;
            for (i = 0; !(newtagset & 1 << i); i++)
                ;
            selmon->pertag->curtag = i + 1;
        }

        /* apply settings for this view */
        selmon->nmaster = selmon->pertag->nmasters[selmon->pertag->curtag];
        selmon->mfact = selmon->pertag->mfacts[selmon->pertag->curtag];
        selmon->sellt = selmon->pertag->sellts[selmon->pertag->curtag];
        selmon->lt[selmon->sellt] =
            selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt];
        selmon->lt[selmon->sellt ^ 1] =
            selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt ^ 1];

        if (selmon->showbar != selmon->pertag->showbars[selmon->pertag->curtag])
            togglebar(NULL);

        focus(NULL);
        arrange(selmon);
    }
}

void view(const Arg *arg) {

    int ui = computeprefix(arg);
    int i;
    printf("%d\n", (int)(arg->ui));

    selmon->seltags ^= 1; /* toggle sel tagset */
    if (ui & tagmask) {
        selmon->tagset[selmon->seltags] = ui & tagmask;

        if (ui == ~0) {
            selmon->pertag->prevtag = selmon->pertag->curtag;
            selmon->pertag->curtag = 0;
        } else {
            for (i = 0; !(ui & 1 << i); i++)
                ;
            if ((i + 1) == selmon->pertag->curtag)
                return;
            selmon->pertag->prevtag = selmon->pertag->curtag;
            selmon->pertag->curtag = i + 1;
        }

        /* apply settings for this view */
        selmon->nmaster = selmon->pertag->nmasters[selmon->pertag->curtag];
        selmon->mfact = selmon->pertag->mfacts[selmon->pertag->curtag];
        selmon->sellt = selmon->pertag->sellts[selmon->pertag->curtag];
        selmon->lt[selmon->sellt] =
            selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt];
        selmon->lt[selmon->sellt ^ 1] =
            selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt ^ 1];

        if (selmon->showbar != selmon->pertag->showbars[selmon->pertag->curtag])
            togglebar(NULL);

        focus(NULL);
        arrange(selmon);
    }
}

void viewtoleft(const Arg *arg) {
    int i;
    if (selmon->pertag->curtag == 1)
        return;
    if (__builtin_popcount(selmon->tagset[selmon->seltags] & tagmask) == 1 &&
        selmon->tagset[selmon->seltags] > 1) {
        selmon->seltags ^= 1; /* toggle sel tagset */
        selmon->tagset[selmon->seltags] =
            selmon->tagset[selmon->seltags ^ 1] >> 1;
        selmon->pertag->prevtag = selmon->pertag->curtag;

        if (selmon->tagset[selmon->seltags ^ 1] >> 1 == ~0)
            selmon->pertag->curtag = 0;
        else {
            for (i = 0; !(selmon->tagset[selmon->seltags ^ 1] >> 1 & 1 << i);
                 i++)
                ;
            selmon->pertag->curtag = i + 1;
        }

        selmon->nmaster = selmon->pertag->nmasters[selmon->pertag->curtag];
        selmon->mfact = selmon->pertag->mfacts[selmon->pertag->curtag];
        selmon->sellt = selmon->pertag->sellts[selmon->pertag->curtag];
        selmon->lt[selmon->sellt] =
            selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt];
        selmon->lt[selmon->sellt ^ 1] =
            selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt ^ 1];

        if (selmon->showbar != selmon->pertag->showbars[selmon->pertag->curtag])
            togglebar(NULL);

        focus(NULL);
        arrange(selmon);
    }
}

void moveleft(const Arg *arg) {
    tagtoleft(arg);
    viewtoleft(arg);
}

void shiftview(const Arg *arg) {
    Arg a;
    Client *c;
    unsigned visible = 0;
    int i = arg->i;
    int count = 0;
    int nextseltags, curseltags = selmon->tagset[selmon->seltags];

    do {
        if (i > 0) // left circular shift
            nextseltags = (curseltags << i) | (curseltags >> (numtags - 1 - i));
        else // right circular shift
            nextseltags =
                curseltags >> (-i) | (curseltags << (numtags - 1 + i));

        // Check if tag is visible
        for (c = selmon->clients; c && !visible; c = c->next)
            if (nextseltags & c->tags) {
                visible = 1;
                break;
            }
        i += arg->i;
    } while (!visible && ++count < 10);

    if (count < 10) {
        if (nextseltags & (1 << 20))
            nextseltags = nextseltags ^ (1 << 20);
        a.i = nextseltags;
        view(&a);
    }
}

void viewtoright(const Arg *arg) {
    int i;

    if (selmon->pertag->curtag == 20)
        return;

    if (__builtin_popcount(selmon->tagset[selmon->seltags] & tagmask) == 1 &&
        selmon->tagset[selmon->seltags] & (tagmask >> 1)) {
        selmon->seltags ^= 1; /* toggle sel tagset */
        selmon->tagset[selmon->seltags] = selmon->tagset[selmon->seltags ^ 1]
                                          << 1;

        selmon->pertag->prevtag = selmon->pertag->curtag;

        if (selmon->tagset[selmon->seltags ^ 1] << 1 == ~0)
            selmon->pertag->curtag = 0;
        else {
            for (i = 0; !(selmon->tagset[selmon->seltags ^ 1] << 1 & 1 << i);
                 i++)
                ;
            selmon->pertag->curtag = i + 1;
        }

        selmon->nmaster = selmon->pertag->nmasters[selmon->pertag->curtag];
        selmon->mfact = selmon->pertag->mfacts[selmon->pertag->curtag];
        selmon->sellt = selmon->pertag->sellts[selmon->pertag->curtag];
        selmon->lt[selmon->sellt] =
            selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt];
        selmon->lt[selmon->sellt ^ 1] =
            selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt ^ 1];

        if (selmon->showbar != selmon->pertag->showbars[selmon->pertag->curtag])
            togglebar(NULL);

        focus(NULL);
        arrange(selmon);
    }
}

void moveright(const Arg *arg) {
    tagtoright(arg);
    viewtoright(arg);
}

void overtoggle(const Arg *arg) {
    Client *c;
    c = selmon->sel;
    unsigned int tmptag;
    int showscratch = 0;

    if (!selmon->clients ||
        (selmon->clients == selmon->overlay && !selmon->overlay->next)) {
        if (selmon->pertag->curtag == 0)
            lastview(NULL);
        return;
    }

    if (selmon->scratchvisible) {
        for (c = selmon->clients; c; c = c->next) {
            if (c->tags & 1 << 20) {
                showscratch = 1;
                break;
            }
        }
        if (showscratch)
            togglescratchpad(NULL);
    }
    if (selmon->fullscreen)
        tempfullscreen(NULL);
    if (selmon->pertag->curtag == 0) {
        tmptag = selmon->pertag->prevtag;
        restoreallfloating(selmon);
        winview(NULL);
    } else {
        tmptag = selmon->pertag->curtag;
        saveallfloating(selmon);
        selmon->lt[selmon->sellt] = selmon->pertag->ltidxs[0][selmon->sellt] =
            (Layout *)&layouts[6];
        view(arg);
        if (selmon->lt[selmon->sellt] != (Layout *)&layouts[6])
            setlayout(&((Arg){.v = &layouts[6]}));
        focus(c);
    }
    selmon->pertag->prevtag = tmptag;
}

void lastview(const Arg *arg) {
    if (selmon->pertag->curtag == selmon->pertag->prevtag)
        focuslastclient(NULL);
    else
        view(&((Arg){.ui = 1 << (selmon->pertag->prevtag - 1)}));
}

// overtoggle but with monocle layout
void fullovertoggle(const Arg *arg) {
    if (selmon->pertag->curtag == 0) {
        winview(NULL);
    } else {
        selmon->lt[selmon->sellt] = selmon->pertag->ltidxs[0][selmon->sellt] =
            (Layout *)&layouts[3];
        view(arg);
    }
}

void winview(const Arg *arg) {
    Window win, win_r, win_p, *win_c;
    unsigned nc;
    int unused;
    Client *c;
    Arg a;

    if (&overviewlayout == selmon->lt[selmon->sellt]->arrange) {
        for (c = selmon->clients; c; c = c->next) {
            if (c == selmon->overlay)
                continue;
            if (c->isfloating)
                restorefloating(c);
        }
    }

    if (!XGetInputFocus(dpy, &win, &unused))
        return;
    while (XQueryTree(dpy, win, &win_r, &win_p, &win_c, &nc) && win_p != win_r)
        win = win_p;

    if (!(c = wintoclient(win)))
        return;

    a.ui = c->tags;
    if (c->tags == 1 << 20) {
        view(&((Arg){.ui = 1 << (selmon->pertag->curtag - 1)}));
    } else {
        view(&a);
    }
    focus(c);
}

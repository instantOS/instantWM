/* See LICENSE file for copyright and license details.
 *
 * dynamic window manager is designed like any other X client as well. It is
 * driven through handling X events. In contrast to other X clients, a window
 * manager selects for SubstructureRedirectMask on the root window, to receive
 * events about window (dis-)appearance. Only one X connection at a time is
 * allowed to select for this event mask.
 *
 * The event handlers of instantWM are organized in an array which is accessed
 * whenever a new event has been fetched. This allows event dispatching
 * in O(1) time.
 *
 * Each child of the root window is called a client, except windows which have
 * set the override_redirect flag. Clients are organized in a linked client
 * list on each monitor, the focus history is remembered through a stack list
 * on each monitor. Each client contains a bit array to indicate the tags of a
 * client.
 *
 * Keys and tagging rules are organized as arrays and defined in config.h.
 *
 * To understand everything else, start reading main().
 */

#include <X11/X.h>
#include <X11/Xlib.h>
#include <X11/Xresource.h>
#include <errno.h>
#include <locale.h>
#include <math.h>
#include <signal.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "instantwm.h"
#include "layouts.h"
#include "util.h"

/* configuration, allows nested code to access above variables */
#include "config.h"

/* variables */
static Systray *systray = NULL;
static const char broken[] = "broken";
static char stext[1024];

static int showalttag = 0;
static int freealttab = 0;

static Client *lastclient;

static int tagprefix = 0;
static int bardragging = 0;
static int altcursor = 0;
static int tagwidth = 0;
static int doubledraw = 0;
static int desktopicons = 0;
static int newdesktop = 0;
static int pausedraw = 0;

static int statuswidth = 0;

static int isdesktop = 0;

static int screen;
static int sw, sh; /* X display screen geometry width, height */
int bh, blw = 0;   /* bar geometry */
static int lrpad;  /* sum of left and right padding for text */
static int (*xerrorxlib)(Display *, XErrorEvent *);
static unsigned int numlockmask = 0;
void (*handler[LASTEvent])(XEvent *) = {[ButtonPress] = buttonpress,
                                        [ButtonRelease] = keyrelease,
                                        [ClientMessage] = clientmessage,
                                        [ConfigureRequest] = configurerequest,
                                        [ConfigureNotify] = configurenotify,
                                        [DestroyNotify] = destroynotify,
                                        [EnterNotify] = enternotify,
                                        [Expose] = expose,
                                        [FocusIn] = focusin,
                                        [KeyRelease] = keyrelease,
                                        [KeyPress] = keypress,
                                        [MappingNotify] = mappingnotify,
                                        [MapRequest] = maprequest,
                                        [MotionNotify] = motionnotify,
                                        [PropertyNotify] = propertynotify,
                                        [ResizeRequest] = resizerequest,
                                        [UnmapNotify] = unmapnotify};
static Atom wmatom[WMLast], netatom[NetLast], xatom[XLast], motifatom;
static int running = 1;
static Cur *cursor[CurLast];

static Clr **scheme;
static Clr ***tagscheme;
static Clr ***windowscheme;
static Clr ***closebuttonscheme;
static Clr *borderscheme;
static Clr *statusscheme;

Display *dpy;
static Drw *drw;
static Monitor *mons;
static Window root, wmcheckwin;
static int focusfollowsmouse = 1;
static int focusfollowsfloatmouse = 1;
static int barleavestatus = 0;
int animated = 1;
int specialnext = 0;

Client *animclient;

int commandoffsets[20];

int forceresize = 0;
Monitor *selmon;

struct Pertag {
    unsigned int curtag, prevtag;   /* current and previous tag */
    int nmasters[LENGTH(tags) + 1]; /* number of windows in master area */
    float mfacts[LENGTH(tags) + 1]; /* mfacts per tag */
    unsigned int sellts[LENGTH(tags) + 1]; /* selected layouts */
    const Layout
        *ltidxs[LENGTH(tags) + 1][2]; /* matrix of tags and layouts indexes  */
    int showbars[LENGTH(tags) + 1];   /* display bar for the current tag */
};

/* compile-time check if all tags fit into an unsigned int bit array. */
struct NumTags {
    char limitexceeded[LENGTH(tags) > 31 ? -1 : 1];
};

void keyrelease(XEvent *e) {}

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

void createdesktop() {
    Client *c;
    Monitor *m;
    m = selmon;
    for (c = m->clients; c; c = c->next) {
        if (strstr(c->name, "ROX-Filer") != NULL) {
            if (c->w > drw->w - 100) {
                focus(c);
                desktopset();
                break;
            }
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

void tempfullscreen() {
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

void createoverlay() {
    Monitor *m;
    Monitor *tm;
    if (!selmon->sel)
        return;
    if (selmon->sel == selmon->fullscreen)
        tempfullscreen();
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
    showoverlay();
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

void showoverlay() {
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
        case 0:
            resize(c, selmon->mx + 20, selmon->my + yoffset - c->h,
                   selmon->ww - 40, c->h, True);
            break;
        case 1:
            resize(c, selmon->mx + selmon->mw - 20, selmon->my + 40, c->w,
                   selmon->mh - 80, True);
            break;
        case 2:
            resize(c, selmon->mx + 20, selmon->my + selmon->mh, selmon->ww - 40,
                   c->h, True);
            break;
        case 3:
            resize(c, selmon->mx - c->w + 20, selmon->my + 40, c->w,
                   selmon->mh - 80, True);
            break;
        default:
            selmon->overlaymode = 0;
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
        case 0:
            animateclient(c, c->x, selmon->my + yoffset, 0, 0, 15, 0);
            break;
        case 1:
            animateclient(c, selmon->mx + selmon->mw - c->w, selmon->my + 40, 0,
                          0, 15, 0);
            break;
        case 2:
            animateclient(c, selmon->mx + 20, selmon->my + selmon->mh - c->h, 0,
                          0, 15, 0);
            break;
        case 3:
            animateclient(c, selmon->mx, selmon->my + 40, 0, 0, 15, 0);
            break;
        default:
            selmon->overlaymode = 0;
            break;
        }
        c->issticky = 1;
    }

    c->bw = 0;
    /* arrange(selmon); */
    focus(c);
    XRaiseWindow(dpy, c->win);
}

void hideoverlay() {
    if (!overlayexists() || !selmon->overlaystatus)
        return;

    Client *c;
    Monitor *m;
    c = selmon->overlay;
    c->issticky = 0;
    if (c == selmon->fullscreen)
        tempfullscreen();
    if (c->islocked) {
        switch (selmon->overlaymode) {
        case 0:
            animateclient(c, c->x, 0 - c->h, 0, 0, 15, 0);
            break;
        case 1:
            animateclient(c, selmon->mx + selmon->mw, selmon->mx + selmon->mw,
                          0, 0, 15, 0);
            break;
        case 2:
            animateclient(c, c->x, selmon->mh + selmon->my, 0, 0, 15, 0);
            break;
        case 3:
            animateclient(c, selmon->mx - c->w, 40, 0, 0, 15, 0);
            break;
        default:
            selmon->overlaymode = 0;
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

void setoverlay() {

    if (!overlayexists()) {
        return;
    }

    if (!selmon->overlaystatus) {
        showoverlay();
    } else {
        if (ISVISIBLE(selmon->overlay)) {
            hideoverlay();
        } else {
            showoverlay();
        }
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

void desktopset() {
    Client *c = selmon->sel;
    c->isfloating = 0;
    arrange(c->mon);
    resize(c, 0, bh, drw->w, drw->h - bh, 0);
    unmanage(c, 0);
    restack(selmon);
    return;
}

void applyrules(Client *c) {
    const char *class, *instance;
    Monitor *m;
    XClassHint ch = {NULL, NULL};

    /* rule matching */
    c->isfloating = 0;
    c->tags = 0;
    XGetClassHint(dpy, c->win, &ch);
    class = ch.res_class ? ch.res_class : broken;
    instance = ch.res_name ? ch.res_name : broken;

    if (specialnext) {
        switch (specialnext) {
        case 1:
            c->isfloating = 1;
            break;
        }
        specialnext = 0;
    } else {
        unsigned int i;
        const Rule *r;
        for (i = 0; i < LENGTH(rules); i++) {
            r = &rules[i];
            if ((!r->title || strstr(c->name, r->title)) &&
                (!r->class || strstr(class, r->class)) &&
                (!r->instance || strstr(instance, r->instance))) {
                if (strstr(r->class, "ROX-Filer") != NULL) {
                    desktopicons = 1;
                    newdesktop = 1;
                }

                if (strstr(r->class, "Onboard") != NULL) {
                    c->issticky = 1;
                }

                switch (r->isfloating) {
                case 2:
                    selmon->sel = c;
                    c->isfloating = 1;
                    centerwindow();
                    break;
                    ;
                case 3:
                    // fullscreen overlay
                    selmon->sel = c;
                    c->isfloating = 1;
                    c->w = c->mon->mw;
                    c->h = c->mon->wh - (selmon->showbar ? bh : 0);
                    if (selmon->showbar)
                        c->y = selmon->my + bh;
                    c->x = selmon->mx;
                    break;
                    ;
                case 4:
                    selmon->sel = c;
                    c->tags = 1 << 20;
                    selmon->scratchvisible = 1;
                    c->issticky = 1;
                    c->isfloating = 1;
                    centerwindow();
                    break;
                    ;
                case 1:
                    c->isfloating = 1;
                    c->y = c->mon->my + (selmon->showbar ? bh : 0);
                    break;
                    ;
                case 0:
                    c->isfloating = 0;
                    break;
                    ;
                }

                c->tags |= r->tags;
                for (m = mons; m && m->num != r->monitor; m = m->next)
                    ;
                if (m)
                    c->mon = m;
            }
        }
    }
    if (ch.res_class)
        XFree(ch.res_class);
    if (ch.res_name)
        XFree(ch.res_name);
    c->tags =
        c->tags & TAGMASK ? c->tags & TAGMASK : c->mon->tagset[c->mon->seltags];
}

int applysizehints(Client *c, int *x, int *y, int *w, int *h, int interact) {
    Monitor *m = c->mon;

    /* set minimum possible */
    *w = MAX(1, *w);
    *h = MAX(1, *h);
    if (interact) {
        if (*x > sw)
            *x = sw - WIDTH(c);
        if (*y > sh)
            *y = sh - HEIGHT(c);
        if (*x + *w + 2 * c->bw < 0)
            *x = 0;
        if (*y + *h + 2 * c->bw < 0)
            *y = 0;
    } else {
        if (*x >= m->wx + m->ww)
            *x = m->wx + m->ww - WIDTH(c);
        if (*y >= m->wy + m->wh)
            *y = m->wy + m->wh - HEIGHT(c);
        if (*x + *w + 2 * c->bw <= m->wx)
            *x = m->wx;
        if (*y + *h + 2 * c->bw <= m->wy)
            *y = m->wy;
    }
    if (*h < bh)
        *h = bh;
    if (*w < bh)
        *w = bh;
    if (resizehints || c->isfloating || !c->mon->lt[c->mon->sellt]->arrange) {
        if (!c->hintsvalid)
            updatesizehints(c);
        /* see last two sentences in ICCCM 4.1.2.3 */
        int baseismin = c->basew == c->minw && c->baseh == c->minh;
        if (!baseismin) { /* temporarily remove base dimensions */
            *w -= c->basew;
            *h -= c->baseh;
        }
        /* adjust for aspect limits */
        if (c->mina > 0 && c->maxa > 0) {
            if (c->maxa < (float)*w / *h)
                *w = *h * c->maxa + 0.5;
            else if (c->mina < (float)*h / *w)
                *h = *w * c->mina + 0.5;
        }
        if (baseismin) { /* increment calculation requires this */
            *w -= c->basew;
            *h -= c->baseh;
        }
        /* adjust for increment value */
        if (c->incw)
            *w -= *w % c->incw;
        if (c->inch)
            *h -= *h % c->inch;
        /* restore base dimensions */
        *w = MAX(*w + c->basew, c->minw);
        *h = MAX(*h + c->baseh, c->minh);
        if (c->maxw)
            *w = MIN(*w, c->maxw);
        if (c->maxh)
            *h = MIN(*h, c->maxh);
    }
    return *x != c->x || *y != c->y || *w != c->w || *h != c->h;
}

void arrange(Monitor *m) {
    resetcursor();
    if (m)
        showhide(m->stack);
    else
        for (m = mons; m; m = m->next)
            showhide(m->stack);
    if (m) {
        arrangemon(m);
        restack(m);
    } else
        for (m = mons; m; m = m->next) {
            arrangemon(m);
        }
}

void arrangemon(Monitor *m) {

    Client *c;
    m->clientcount = clientcountmon(m);

    for (c = nexttiled(m->clients); c; c = nexttiled(c->next)) {
        if (!c->isfloating && !c->isfullscreen &&
            ((c->mon->clientcount == 1 &&
              NULL != c->mon->lt[c->mon->sellt]->arrange) ||
             &monocle == c->mon->lt[c->mon->sellt]->arrange)) {
            savebw(c);
            c->bw = 0;
        } else {
            restorebw(c);
        }
    }

    strncpy(m->ltsymbol, m->lt[m->sellt]->symbol, sizeof m->ltsymbol);
    if (m->lt[m->sellt]->arrange)
        m->lt[m->sellt]->arrange(m);
    else
        floatl(m);

    if (m->fullscreen) {
        int tbw;
        tbw = selmon->fullscreen->bw;
        if (m->fullscreen->isfloating)
            savefloating(selmon->fullscreen);
        resize(m->fullscreen, m->mx, m->my + (m->showbar * bh),
               m->mw - (tbw * 2), m->mh - (m->showbar * bh) - (tbw * 2), 0);
    }
}

void attach(Client *c) {
    c->next = c->mon->clients;
    c->mon->clients = c;
}

void attachstack(Client *c) {
    c->snext = c->mon->stack;
    c->mon->stack = c;
}

void resetcursor() {
    if (!altcursor)
        return;
    XDefineCursor(dpy, root, cursor[CurNormal]->cursor);
    altcursor = 0;
}

void buttonpress(XEvent *e) {
    unsigned int i, x, click, occ = 0;
    Arg arg = {0};
    Client *c;
    Monitor *m;
    XButtonPressedEvent *ev = &e->xbutton;

    click = ClkRootWin;
    /* focus monitor if necessary */
    if ((m = wintomon(ev->window)) && m != selmon) {
        /* if focus doesn't follow the mouse, the scroll wheel shouldn't switch
         * focus */
        if (focusfollowsmouse || ev->button <= Button3) {
            unfocus(selmon->sel, 1);
            selmon = m;
            focus(NULL);
        }
    }

    if (ev->window == selmon->barwin) {
        i = 0;
        x = startmenusize;
        for (c = m->clients; c; c = c->next)
            occ |= c->tags == 255 ? 0 : c->tags;
        do {
            /* do not reserve space for vacant tags */
            if (i >= 9)
                continue;
            if (selmon->showtags) {
                if (!(occ & 1 << i || m->tagset[m->seltags] & 1 << i))
                    continue;
            }

            x += TEXTW(tags[i]);
        } while (ev->x >= x && ++i < LENGTH(tags));
        if (ev->x < startmenusize) {
            click = ClkStartMenu;
            selmon->gesture = 0;
            drawbar(selmon);
        } else if (i < LENGTH(tags)) {
            click = ClkTagBar;
            arg.ui = 1 << i;
        } else if (ev->x < x + blw)
            click = ClkLtSymbol;
        else if (!selmon->sel && ev->x > x + blw && ev->x < x + blw + bh)
            click = ClkShutDown;
        /* 2px right padding */
        else if (ev->x >
                 selmon->ww - getsystraywidth() - statuswidth + lrpad - 2)
            click = ClkStatusText;
        else {
            if (selmon->stack) {
                x += blw;
                c = m->clients;

                do {
                    if (!ISVISIBLE(c))
                        continue;
                    else
                        x += (1.0 / (double)m->bt) * m->btw;
                } while (ev->x > x && (c = c->next));

                if (c) {
                    arg.v = c;
                    if (c != selmon->sel ||
                        ev->x > x - (1.0 / (double)m->bt) * m->btw + 32) {
                        click = ClkWinTitle;
                    } else {
                        click = ClkCloseButton;
                    }
                }
            } else {
                click = ClkRootWin;
            }
        }
    } else if ((c = wintoclient(ev->window))) {
        if (focusfollowsmouse || ev->button <= Button3) {
            focus(c);
            restack(selmon);
        }
        XAllowEvents(dpy, ReplayPointer, CurrentTime);
        click = ClkClientWin;
    } else if (ev->x > selmon->mx + selmon->mw - 50) {
        click = ClkSideBar;
    }
    for (i = 0; i < LENGTH(buttons); i++)
        if (click == buttons[i].click && buttons[i].func &&
            buttons[i].button == ev->button &&
            CLEANMASK(buttons[i].mask) == CLEANMASK(ev->state))
            buttons[i].func((click == ClkTagBar || click == ClkWinTitle ||
                             click == ClkCloseButton || click == ClkShutDown ||
                             click == ClkSideBar) &&
                                    buttons[i].arg.i == 0
                                ? &arg
                                : &buttons[i].arg);
}

void checkotherwm(void) {
    xerrorxlib = XSetErrorHandler(xerrorstart);
    /* this causes an error if some other window manager is running */
    XSelectInput(dpy, DefaultRootWindow(dpy), SubstructureRedirectMask);
    XSync(dpy, False);
    XSetErrorHandler(xerror);
    XSync(dpy, False);
}

void cleanup(void) {
    Arg a = {.ui = ~0};
    Layout foo = {"", NULL};
    Monitor *m;
    size_t i;
    size_t u;

    view(&a);
    selmon->lt[selmon->sellt] = &foo;
    for (m = mons; m; m = m->next)
        while (m->stack)
            unmanage(m->stack, 0);
    XUngrabKey(dpy, AnyKey, AnyModifier, root);
    while (mons)
        cleanupmon(mons);
    if (showsystray) {
        XUnmapWindow(dpy, systray->win);
        XDestroyWindow(dpy, systray->win);
        free(systray);
    }
    for (i = 0; i < CurLast; i++)
        drw_cur_free(drw, cursor[i]);

    for (i = 0; i < LENGTH(tagcolors); i++) {
        for (u = 0; u < LENGTH(tagcolors[i]); u++) {
            free(tagscheme[i][u]);
        }
    }

    for (i = 0; i < LENGTH(windowcolors); i++) {
        for (u = 0; u < LENGTH(windowcolors[i]); u++) {
            free(windowscheme[i][u]);
        }
    }

    for (i = 0; i < LENGTH(closebuttoncolors); i++) {
        for (u = 0; u < LENGTH(closebuttoncolors[i]); u++) {
            free(closebuttonscheme[i][u]);
        }
    }

    free(statusscheme);
    free(borderscheme);

    //TODO figure out how to do this with the custom theming code (this only frees dwm schemes)
    /* for (i = 0; i < LENGTH(colors) + 1; i++) */
    /*     free(scheme[i]); */
    // free(scheme)
    XDestroyWindow(dpy, wmcheckwin);
    drw_free(drw);
    XSync(dpy, False);
    XSetInputFocus(dpy, PointerRoot, RevertToPointerRoot, CurrentTime);
    XDeleteProperty(dpy, root, netatom[NetActiveWindow]);
}

void cleanupmon(Monitor *mon) {
    Monitor *m;

    if (mon == mons)
        mons = mons->next;
    else {
        for (m = mons; m && m->next != mon; m = m->next)
            ;
        m->next = mon->next;
    }
    XUnmapWindow(dpy, mon->barwin);
    XDestroyWindow(dpy, mon->barwin);
    free(mon);
}

void clientmessage(XEvent *e) {
    XWindowAttributes wa;
    XSetWindowAttributes swa;
    XClientMessageEvent *cme = &e->xclient;
    Client *c = wintoclient(cme->window);
    unsigned int i;

    if (showsystray && cme->window == systray->win &&
        cme->message_type == netatom[NetSystemTrayOP]) {
        /* add systray icons */
        if (cme->data.l[1] == SYSTEM_TRAY_REQUEST_DOCK) {
            if (!(c = (Client *)calloc(1, sizeof(Client))))
                die("fatal: could not malloc() %u bytes\n", sizeof(Client));
            if (!(c->win = cme->data.l[2])) {
                free(c);
                return;
            }
            c->mon = selmon;
            c->next = systray->icons;
            systray->icons = c;
            XGetWindowAttributes(dpy, c->win, &wa);
            c->x = c->oldx = c->y = c->oldy = 0;
            c->w = c->oldw = wa.width;
            c->h = c->oldh = wa.height;
            c->oldbw = wa.border_width;
            c->bw = 0;
            c->isfloating = True;
            /* reuse tags field as mapped status */
            c->tags = 1;
            updatesizehints(c);
            updatesystrayicongeom(c, wa.width, wa.height);
            XAddToSaveSet(dpy, c->win);
            XSelectInput(dpy, c->win,
                         StructureNotifyMask | PropertyChangeMask |
                             ResizeRedirectMask);
            XReparentWindow(dpy, c->win, systray->win, 0, 0);
            /* use parents background color */
            swa.background_pixel = statusscheme[ColBg].pixel;
            XChangeWindowAttributes(dpy, c->win, CWBackPixel, &swa);
            sendevent(c->win, netatom[Xembed], StructureNotifyMask, CurrentTime,
                      XEMBED_EMBEDDED_NOTIFY, 0, systray->win,
                      XEMBED_EMBEDDED_VERSION);
            /* FIXME not sure if I have to send these events, too */
            sendevent(c->win, netatom[Xembed], StructureNotifyMask, CurrentTime,
                      XEMBED_FOCUS_IN, 0, systray->win,
                      XEMBED_EMBEDDED_VERSION);
            sendevent(c->win, netatom[Xembed], StructureNotifyMask, CurrentTime,
                      XEMBED_WINDOW_ACTIVATE, 0, systray->win,
                      XEMBED_EMBEDDED_VERSION);
            sendevent(c->win, netatom[Xembed], StructureNotifyMask, CurrentTime,
                      XEMBED_MODALITY_ON, 0, systray->win,
                      XEMBED_EMBEDDED_VERSION);
            XSync(dpy, False);
            resizebarwin(selmon);
            updatesystray();
            setclientstate(c, NormalState);
        }
        return;
    }
    if (!c)
        return;
    if (cme->message_type == netatom[NetWMState]) {
        if (cme->data.l[1] == netatom[NetWMFullscreen] ||
            cme->data.l[2] == netatom[NetWMFullscreen])
            setfullscreen(c,
                          (cme->data.l[0] == 1 /* _NET_WM_STATE_ADD    */
                           || (cme->data.l[0] == 2 /* _NET_WM_STATE_TOGGLE */ &&
                               (!c->isfullscreen || c->isfakefullscreen))));
    } else if (cme->message_type == netatom[NetActiveWindow]) {
        if (c == c->mon->overlay) {
            if (c->mon != selmon) {
                unfocus(selmon->sel, 0);
                selmon = c->mon;
                focus(NULL);
            }
            showoverlay();
        } else if (c->tags == 1 << 20) {
            selmon = c->mon;
            togglescratchpad(NULL);
        } else {
            if (HIDDEN(c))
                show(c);
            for (i = 0; i < LENGTH(tags) && !((1 << i) & c->tags); i++)
                ;
            if (i < LENGTH(tags)) {
                const Arg a = {.ui = 1 << i};
                if (selmon != c->mon) {
                    unfocus(selmon->sel, 0);
                    selmon = c->mon;
                }
                view(&a);
                focus(c);
                restack(selmon);
            }
        }
    }
}

void configure(Client *c) {
    XConfigureEvent ce;

    ce.type = ConfigureNotify;
    ce.display = dpy;
    ce.event = c->win;
    ce.window = c->win;
    ce.x = c->x;
    ce.y = c->y;
    ce.width = c->w;
    ce.height = c->h;
    ce.border_width = c->bw;
    ce.above = None;
    ce.override_redirect = False;
    XSendEvent(dpy, c->win, False, StructureNotifyMask, (XEvent *)&ce);
}

void configurenotify(XEvent *e) {
    Monitor *m;
    Client *c;
    XConfigureEvent *ev = &e->xconfigure;

    /* TODO: updategeom handling sucks, needs to be simplified */
    if (ev->window == root) {
        int dirty = (sw != ev->width || sh != ev->height);
        sw = ev->width;
        sh = ev->height;
        if (updategeom() || dirty) {
            drw_resize(drw, sw, bh);
            updatebars();
            for (m = mons; m; m = m->next) {
                for (c = m->clients; c; c = c->next) {
                    if (c->isfakefullscreen)
                        XMoveResizeWindow(dpy, m->barwin, m->wx, m->by, m->ww,
                                          bh);
                    else if (c->isfullscreen)
                        resizeclient(c, m->mx, m->my, m->mw, m->mh);
                }
                resizebarwin(m);
            }
            focus(NULL);
            arrange(NULL);
        }
    }
}

void distributeclients(const Arg *arg) {
    Client *c;
    int tagcounter = 0;
    focus(NULL);

    for (c = selmon->clients; c; c = c->next) {
        // overlays or scratchpads aren't on regular tags anyway
        if (c == selmon->overlay || c->tags & 1 << 20)
            continue;
        if (tagcounter > 8) {
            tagcounter = 0;
        }
        if (c && 1 << tagcounter & TAGMASK) {
            c->tags = 1 << tagcounter & TAGMASK;
        }
        tagcounter++;
    }
    focus(NULL);
    arrange(selmon);
}

void configurerequest(XEvent *e) {
    Client *c;
    Monitor *m;
    XConfigureRequestEvent *ev = &e->xconfigurerequest;
    XWindowChanges wc;

    if ((c = wintoclient(ev->window))) {
        if (ev->value_mask & CWBorderWidth)
            c->bw = ev->border_width;
        else if (c->isfloating || !selmon->lt[selmon->sellt]->arrange) {
            m = c->mon;
            if (ev->value_mask & CWX) {
                c->oldx = c->x;
                c->x = m->mx + ev->x;
            }
            if (ev->value_mask & CWY) {
                c->oldy = c->y;
                c->y = m->my + ev->y;
            }
            if (ev->value_mask & CWWidth) {
                c->oldw = c->w;
                c->w = ev->width;
            }
            if (ev->value_mask & CWHeight) {
                c->oldh = c->h;
                c->h = ev->height;
            }
            if ((c->x + c->w) > m->mx + m->mw && c->isfloating)
                c->x = m->mx +
                       (m->mw / 2 - WIDTH(c) / 2); /* center in x direction */
            if ((c->y + c->h) > m->my + m->mh && c->isfloating)
                c->y = m->my +
                       (m->mh / 2 - HEIGHT(c) / 2); /* center in y direction */
            if ((ev->value_mask & (CWX | CWY)) &&
                !(ev->value_mask & (CWWidth | CWHeight)))
                configure(c);
            if (ISVISIBLE(c))
                XMoveResizeWindow(dpy, c->win, c->x, c->y, c->w, c->h);
        } else
            configure(c);
    } else {
        wc.x = ev->x;
        wc.y = ev->y;
        wc.width = ev->width;
        wc.height = ev->height;
        wc.border_width = ev->border_width;
        wc.sibling = ev->above;
        wc.stack_mode = ev->detail;
        XConfigureWindow(dpy, ev->window, ev->value_mask, &wc);
    }
    XSync(dpy, False);
}

Monitor *createmon(void) {
    Monitor *m;
    unsigned int i;

    m = ecalloc(1, sizeof(Monitor));
    m->tagset[0] = m->tagset[1] = 1;
    m->mfact = mfact;
    m->nmaster = nmaster;
    m->showbar = showbar;
    m->topbar = topbar;
    m->clientcount = 0;
    m->overlaymode = 0;
    m->scratchvisible = 0;
    m->lt[0] = &layouts[3];
    m->lt[1] = &layouts[0];
    strncpy(m->ltsymbol, layouts[0].symbol, sizeof m->ltsymbol);
    m->pertag = ecalloc(1, sizeof(Pertag));
    m->pertag->curtag = m->pertag->prevtag = 1;

    for (i = 0; i <= LENGTH(tags); i++) {
        m->pertag->nmasters[i] = m->nmaster;
        m->pertag->mfacts[i] = m->mfact;

        m->pertag->ltidxs[i][0] = m->lt[1];
        m->pertag->ltidxs[i][1] = m->lt[0];
        m->pertag->sellts[i] = m->sellt;

        m->pertag->showbars[i] = m->showbar;
    }

    return m;
}

void cyclelayout(const Arg *arg) {
    Layout *l;
    for (l = (Layout *)layouts; l != selmon->lt[selmon->sellt]; l++)
        ;
    if (arg->i > 0) {
        if (l->symbol && (l + 1)->symbol) {
            if ((l + 1)->arrange == &overviewlayout)
                setlayout(&((Arg){.v = (l + 2)}));
            else
                setlayout(&((Arg){.v = (l + 1)}));
        } else {
            setlayout(&((Arg){.v = layouts}));
        }
    } else {
        if (l != layouts && (l - 1)->symbol) {
            if ((l - 1)->arrange == &overviewlayout)
                setlayout(&((Arg){.v = (l - 2)}));
            else
                setlayout(&((Arg){.v = (l - 1)}));
        } else {
            setlayout(&((Arg){.v = &layouts[LENGTH(layouts) - 2]}));
        }
    }
}

void destroynotify(XEvent *e) {
    Client *c;
    XDestroyWindowEvent *ev = &e->xdestroywindow;

    if ((c = wintoclient(ev->window)))
        unmanage(c, 1);
    else if ((c = wintosystrayicon(ev->window))) {
        removesystrayicon(c);
        resizebarwin(selmon);
        updatesystray();
    }
}

void detach(Client *c) {
    Client **tc;

    for (tc = &c->mon->clients; *tc && *tc != c; tc = &(*tc)->next)
        ;
    *tc = c->next;
}

void detachstack(Client *c) {
    Client **tc, *t;

    for (tc = &c->mon->stack; *tc && *tc != c; tc = &(*tc)->snext)
        ;
    *tc = c->snext;

    if (c == c->mon->sel) {
        for (t = c->mon->stack; t && !ISVISIBLE(t); t = t->snext)
            ;
        c->mon->sel = t;
    }
}

Monitor *dirtomon(int dir) {
    Monitor *m = NULL;

    if (dir > 0) {
        if (!(m = selmon->next))
            m = mons;
    } else if (selmon == mons)
        for (m = mons; m->next; m = m->next)
            ;
    else
        for (m = mons; m->next != selmon; m = m->next)
            ;
    return m;
}

void clickstatus(const Arg *arg) {
    int x, y, i;
    getrootptr(&x, &y);
    i = 0;
    while (1) {
        if (i > 19 || (commandoffsets[i] == -1) || (commandoffsets[i] == 0))
            break;
        if (x - selmon->mx < commandoffsets[i])
            break;
        i++;
    }
    fprintf(stderr, "\ncounter: %d, x: %d, offset: %d", i, x - selmon->mx,
            commandoffsets[i]);
}

int drawstatusbar(Monitor *m, int bh, char *stext) {
    int ret, i, w, x, len, cmdcounter;
    short isCode = 0;
    char *text;
    char *p;

    len = strlen(stext) + 1;
    if (!(text = (char *)malloc(sizeof(char) * len)))
        die("malloc");
    p = text;
    memcpy(text, stext, len);

    /* compute width of the status text */
    w = 0;
    i = -1;
    while (text[++i]) {
        if (text[i] == '^') {
            if (!isCode) {
                isCode = 1;
                text[i] = '\0';
                w += TEXTW(text) - lrpad;
                text[i] = '^';
                if (text[++i] == 'f')
                    w += atoi(text + ++i);
            } else {
                isCode = 0;
                text = text + i + 1;
                i = -1;
            }
        }
    }
    if (!isCode)
        w += TEXTW(text) - lrpad;
    else
        isCode = 0;
    text = p;
    statuswidth = w;
    w += 2; /* 1px padding on both sides */
    ret = x = m->ww - w - getsystraywidth();

    drw_setscheme(drw, statusscheme);
    drw_rect(drw, x, 0, w, bh, 1, 1);
    x++;

    /* process status text */
    i = -1;
    cmdcounter = 0;

    int customcolor = 0;

    while (text[++i]) {
        if (text[i] == '^' && !isCode) {
            isCode = 1;

            text[i] = '\0';
            w = TEXTW(text) - lrpad;
            drw_text(drw, x, 0, w, bh, 0, text, 0, 0);

            x += w;

            /* process code */
            while (text[++i] != '^') {
                if (text[i] == 'c') {
                    char buf[8];
                    memcpy(buf, (char *)text + i + 1, 7);
                    buf[7] = '\0';
                    customcolor = 1;
                    drw_clr_create(drw, &drw->scheme[ColBg], buf);
                    i += 7;
                } else if (text[i] == 't') {
                    char buf[8];
                    memcpy(buf, (char *)text + i + 1, 7);
                    buf[7] = '\0';
                    customcolor = 1;
                    drw_clr_create(drw, &drw->scheme[ColFg], buf);
                    i += 7;
                } else if (text[i] == 'd') {
                    drw_clr_create(drw, &drw->scheme[ColBg],
                                   statusbarcolors[ColBg]);
                    drw_clr_create(drw, &drw->scheme[ColFg],
                                   statusbarcolors[ColFg]);
                } else if (text[i] == 'r') {
                    int rx = atoi(text + ++i);
                    while (text[++i] != ',')
                        ;
                    int ry = atoi(text + ++i);
                    while (text[++i] != ',')
                        ;
                    int rw = atoi(text + ++i);
                    while (text[++i] != ',')
                        ;
                    int rh = atoi(text + ++i);

                    drw_rect(drw, rx + x, ry, rw, rh, 1, 0);
                } else if (text[i] == 'f') {
                    x += atoi(text + ++i);
                } else if (text[i] == 'o') {
                    if (cmdcounter <= 20) {
                        commandoffsets[cmdcounter] = x;
                        cmdcounter++;
                    }
                }
            }

            text = text + i + 1;
            i = -1;
            isCode = 0;
        }
    }

    if (customcolor) {
        drw_clr_create(drw, &drw->scheme[ColBg], statusbarcolors[ColBg]);
        drw_clr_create(drw, &drw->scheme[ColFg], statusbarcolors[ColFg]);
    }

    if (cmdcounter < 20) {
        if (cmdcounter == 0)
            commandoffsets[0] = -1;
        else
            commandoffsets[cmdcounter + 1] = -1;
    }

    cmdcounter = 0;
    while (1) {
        if (cmdcounter > 19 || (commandoffsets[cmdcounter] == -1) ||
            (commandoffsets[cmdcounter] == 0))
            break;
        cmdcounter++;
    }

    if (!isCode) {
        w = TEXTW(text) - lrpad;
        drw_text(drw, x, 0, w, bh, 0, text, 0, 0);
    }

    drw_setscheme(drw, statusscheme);
    free(p);

    return ret;
}

void drawbar(Monitor *m) {
    if (pausedraw)
        return;

    int x, w, sw = 0, n = 0, stw = 0, roundw, iconoffset, ishover;

    unsigned int i, occ = 0, urg = 0;
    Client *c;

    if (!m->showbar)
        return;

    if (showsystray && m == systraytomon(m))
        stw = getsystraywidth();

    /* draw status first so it can be overdrawn by tags later */
    if (m == selmon) { /* status is only drawn on selected monitor */
        sw = m->ww - stw - drawstatusbar(m, bh, stext);
    }

    // draw start menu icon with instantOS logo
    if (tagprefix)
        drw_setscheme(drw, tagscheme[SchemeNoHover][SchemeTagFocus]);
    else
        drw_setscheme(drw, statusscheme);

    iconoffset = (bh - 20) / 2;
    int startmenuinvert = (selmon->gesture == 13);
    drw_rect(drw, 0, 0, startmenusize, bh, 1, startmenuinvert ? 0 : 1);
    drw_rect(drw, 5, iconoffset, 14, 14, 1, startmenuinvert ? 1 : 0);
    drw_rect(drw, 9, iconoffset + 4, 6, 6, 1, startmenuinvert ? 0 : 1);
    drw_rect(drw, 19, iconoffset + 14, 6, 6, 1, startmenuinvert ? 1 : 0);

    resizebarwin(m);

    // check for clients on tag
    for (c = m->clients; c; c = c->next) {
        if (ISVISIBLE(c))
            n++;
        occ |= c->tags == 255 ? 0 : c->tags;

        if (c->isurgent)
            urg |= c->tags;
    }

    x = startmenusize;

    // render all tag indicators
    for (i = 0; i < LENGTH(tags); i++) {
        ishover = i == selmon->gesture - 1 ? SchemeHover : SchemeNoHover;
        if (i >= 9)
            continue;
        if (i == 8 && selmon->pertag->curtag > 9)
            i = selmon->pertag->curtag - 1;

        /* do not draw vacant tags */
        if (selmon->showtags) {
            if (!(occ & 1 << i || m->tagset[m->seltags] & 1 << i))
                continue;
        }

        w = TEXTW(tags[i]);

        // tag has client
        if (occ & 1 << i) {
            if (m == selmon && selmon->sel && selmon->sel->tags & 1 << i) {
                drw_setscheme(drw, tagscheme[ishover][SchemeTagFocus]);
            } else {
                // tag is active, has clients but is not in focus
                if (m->tagset[m->seltags] & 1 << i) {
                    drw_setscheme(drw, tagscheme[ishover][SchemeTagNoFocus]);
                } else {
                    // do not color tags if vacant tags are hidden
                    if (!selmon->showtags) {
                        drw_setscheme(drw, tagscheme[ishover][SchemeTagFilled]);
                    } else {
                        drw_setscheme(drw,
                                      tagscheme[ishover][SchemeTagInactive]);
                    }
                }
            }
        } else { // tag does not have a client
            if (m->tagset[m->seltags] & 1 << i) {
                drw_setscheme(drw, tagscheme[ishover][SchemeTagEmpty]);
            } else {
                drw_setscheme(drw, tagscheme[ishover][SchemeTagInactive]);
            }
        }

        if (i == selmon->gesture - 1) {
            roundw = 8;
            if (bardragging) {
                drw_setscheme(drw, tagscheme[SchemeHover][SchemeTagFilled]);
            }
            drw_text(drw, x, 0, w, bh, lrpad / 2,
                     (showalttag ? tagsalt[i] : tags[i]), urg & 1 << i, roundw);

        } else {
            drw_text(drw, x, 0, w, bh, lrpad / 2,
                     (showalttag ? tagsalt[i] : tags[i]), urg & 1 << i, 4);
        }
        x += w;
    }

    // render layout indicator
    w = blw = 60;
    drw_setscheme(drw, statusscheme);
    x = drw_text(drw, x, 0, w, bh, (w - TEXTW(m->ltsymbol)) * 0.5 + 10,
                 m->ltsymbol, 0, 0);

    if ((w = m->ww - sw - x - stw) > bh) {
        if (n > 0) {
            // render all window titles
            for (c = m->clients; c; c = c->next) {
                if (!ISVISIBLE(c))
                    continue;

                ishover = selmon->hoverclient && !selmon->gesture &&
                                  c == selmon->hoverclient
                              ? SchemeHover
                              : SchemeNoHover;

                if (m->sel == c) {
                    if (c == selmon->overlay) {
                        drw_setscheme(
                            drw, windowscheme[ishover][SchemeWinOverlayFocus]);
                    } else if (c->issticky) {
                        drw_setscheme(
                            drw, windowscheme[ishover][SchemeWinStickyFocus]);
                    } else {
                        drw_setscheme(drw,
                                      windowscheme[ishover][SchemeWinFocus]);
                    }
                } else {
                    if (c == selmon->overlay) {
                        drw_setscheme(drw,
                                      windowscheme[ishover][SchemeWinOverlay]);
                    } else if (c->issticky) {
                        drw_setscheme(drw,
                                      windowscheme[ishover][SchemeWinSticky]);
                    } else if (HIDDEN(c)) {
                        drw_setscheme(
                            drw, windowscheme[ishover][SchemeWinMinimized]);
                    } else {
                        drw_setscheme(drw,
                                      windowscheme[ishover][SchemeWinNormal]);
                    }
                }

                // don't center text if it is too long
                if (TEXTW(c->name) < (1.0 / (double)n) * w - 64) {
                    drw_text(drw, x, 0, (1.0 / (double)n) * w, bh,
                             ((1.0 / (double)n) * w - TEXTW(c->name)) * 0.5,
                             c->name, 0, 4);
                } else {
                    drw_text(drw, x, 0, (1.0 / ((double)n) * w), bh,
                             lrpad / 2 + 20, c->name, 0, 4);
                }

                if (m->sel == c) {
                    // render close button
                    ishover =
                        selmon->gesture != 12 ? SchemeNoHover : SchemeHover;

                    if (c->islocked) {
                        drw_setscheme(
                            drw, closebuttonscheme[ishover][SchemeCloseLocked]);
                    } else if (c == selmon->fullscreen) {
                        drw_setscheme(
                            drw,
                            closebuttonscheme[ishover][SchemeCloseFullscreen]);
                    } else {
                        drw_setscheme(
                            drw, closebuttonscheme[ishover][SchemeCloseNormal]);
                    }

                    XSetForeground(drw->dpy, drw->gc, drw->scheme[ColBg].pixel);
                    XFillRectangle(drw->dpy, drw->drawable, drw->gc, x + bh / 6,
                                   (bh - 20) / 2 - !ishover * 4, 20, 16);
                    XSetForeground(drw->dpy, drw->gc,
                                   drw->scheme[ColDetail].pixel);
                    XFillRectangle(drw->dpy, drw->drawable, drw->gc, x + bh / 6,
                                   (bh - 20) / 2 + 16 - !ishover * 4, 20,
                                   4 + !ishover * 4);

                    // save position of focussed window title on bar
                    m->activeoffset = selmon->mx + x;
                }
                x += (1.0 / (double)n) * w;
            }
        } else {
            drw_setscheme(drw, statusscheme);
            drw_rect(drw, x, 0, w, bh, 1, 1);
            // render shutdown button
            drw_text(drw, x, 0, bh, bh, lrpad / 2, "", 0, 0);
            // display help message if no application is opened
            if (!selmon->clients) {
                int titlewidth =
                    TEXTW("Press space to launch an application") < m->btw
                        ? TEXTW("Press space to launch an application")
                        : (m->btw - bh);
                drw_text(drw, x + bh + ((m->btw - bh) - titlewidth + 1) / 2, 0,
                         titlewidth, bh, 0,
                         "Press space to launch an application", 0, 0);
            }
        }
    }

    drw_setscheme(drw, statusscheme);

    m->bt = n;
    m->btw = w;
    drw_map(drw, m->barwin, 0, 0, m->ww, bh);
}

void drawbars(void) {
    Monitor *m;

    for (m = mons; m; m = m->next)
        drawbar(m);
}

void enternotify(XEvent *e) {
    Client *c;
    Monitor *m;
    XCrossingEvent *ev = &e->xcrossing;
    int resizeexit = 0;
    // deactivate area at the top to prevent overlay gesture from glitching out
    if (barleavestatus && ev->y_root >= selmon->my + 5) {
        resetbar();
        barleavestatus = 0;
    }
    /* Only care about mouse motion if the focus follows the mouse */
    if ((ev->mode != NotifyNormal || ev->detail == NotifyInferior) &&
        ev->window != root)
        return;
    c = wintoclient(ev->window);
    if (c && selmon->sel &&
        (selmon->sel->isfloating || !selmon->lt[selmon->sellt]->arrange) &&
        c != selmon->sel &&
        (ev->window == root || visible(c) || ISVISIBLE(c) ||
         selmon->sel->issticky)) {
        resizeexit = resizeborder(NULL);
        if (focusfollowsfloatmouse) {
            if (!resizeexit)
                return;
            Client *newc = getcursorclient();
            if (newc && newc != selmon->sel)
                c = newc;
        } else {
            return;
        }
    }
    if (!focusfollowsmouse)
        return;
    m = c ? c->mon : wintomon(ev->window);
    if (m != selmon) {
        unfocus(selmon->sel, 1);
        selmon = m;
    } else {
        if (!focusfollowsfloatmouse) {
            if (ev->window != root && selmon->sel && c && c->isfloating &&
                selmon->lt[selmon->sellt] != (Layout *)&layouts[6])
                return;
        }
        if (!c || c == selmon->sel)
            return;
    }
    focus(c);
}

void expose(XEvent *e) {
    Monitor *m;
    XExposeEvent *ev = &e->xexpose;

    if (ev->count == 0 && (m = wintomon(ev->window))) {
        drawbar(m);
        if (m == selmon)
            updatesystray();
    }
}

void focus(Client *c) {
    resetcursor();
    if (!c || !ISVISIBLE(c) || HIDDEN(c))
        for (c = selmon->stack; c && (!ISVISIBLE(c) || HIDDEN(c)); c = c->snext)
            ;
    if (selmon->sel && selmon->sel != c)
        unfocus(selmon->sel, 0);
    if (c) {
        if (c->mon != selmon)
            selmon = c->mon;
        if (c->isurgent)
            seturgent(c, 0);
        detachstack(c);
        attachstack(c);
        grabbuttons(c, 1);
        if (!c->isfloating)
            XSetWindowBorder(dpy, c->win,
                             borderscheme[SchemeBorderTileFocus].pixel);
        else
            XSetWindowBorder(dpy, c->win,
                             borderscheme[SchemeBorderFloatFocus].pixel);

        setfocus(c);
        if (c->tags & 1 << 20) {
            selmon->activescratchpad = c;
        }
    } else {
        XSetInputFocus(dpy, root, RevertToPointerRoot, CurrentTime);
        XDeleteProperty(dpy, root, netatom[NetActiveWindow]);
    }
    selmon->sel = c;
    if (selmon->gesture != 11 && selmon->gesture)
        selmon->gesture = 0;

    if (selmon->gesture < 9)
        selmon->gesture = 0;
    selmon->hoverclient = NULL;

    drawbars();
    if (!c) {
        if (!isdesktop) {
            isdesktop = 1;
            grabkeys();
        }
    } else if (isdesktop) {
        isdesktop = 0;
        grabkeys();
    }
}

/* there are some broken focus acquiring clients needing extra handling */
void focusin(XEvent *e) {
    XFocusChangeEvent *ev = &e->xfocus;

    if (selmon->sel && ev->window != selmon->sel->win)
        setfocus(selmon->sel);
}

void followmon(const Arg *arg) {
    Client *c;
    if (!selmon->sel)
        return;
    c = selmon->sel;
    tagmon(arg);
    selmon = c->mon;
    focus(NULL);
    focus(c);
    XRaiseWindow(dpy, c->win);
    warp(c);
}

void focusmon(const Arg *arg) {
    Monitor *m;

    if (!mons->next)
        return;
    if ((m = dirtomon(arg->i)) == selmon)
        return;
    unfocus(selmon->sel, 0);
    selmon = m;
    focus(NULL);
}

void focusnmon(const Arg *arg) {
    Monitor *m;
    int i;

    if (!mons->next)
        return;

    m = mons;

    for (i = 0; i < arg->i; ++i) {
        if (m->next) {
            m = m->next;
        } else {
            break;
        }
    }

    unfocus(selmon->sel, 0);
    selmon = m;
    focus(NULL);
}

void focusstack(const Arg *arg) {
    Client *c = NULL, *i;

    if (!selmon->sel ||
        (selmon->sel->isfullscreen && !selmon->sel->isfakefullscreen))
        return;
    if (arg->i > 0) {
        for (c = selmon->sel->next; c && (!ISVISIBLE(c) || HIDDEN(c));
             c = c->next)
            ;
        if (!c)
            for (c = selmon->clients; c && (!ISVISIBLE(c) || HIDDEN(c));
                 c = c->next)
                ;
    } else {
        for (i = selmon->clients; i != selmon->sel; i = i->next)
            if (ISVISIBLE(i) && !HIDDEN(i))
                c = i;
        if (!c)
            for (; i; i = i->next)
                if (ISVISIBLE(i) && !HIDDEN(i))
                    c = i;
    }
    if (c) {
        focus(c);
        restack(selmon);
    }
}

Atom getatomprop(Client *c, Atom prop) {
    int di;
    unsigned long dl;
    unsigned char *p = NULL;
    Atom da, atom = None;
    /* FIXME getatomprop should return the number of items and a pointer to
     * the stored data instead of this workaround */
    Atom req = XA_ATOM;
    if (prop == xatom[XembedInfo])
        req = xatom[XembedInfo];

    if (XGetWindowProperty(dpy, c->win, prop, 0L, sizeof atom, False, req, &da,
                           &di, &dl, &dl, &p) == Success &&
        p) {
        atom = *(Atom *)p;
        if (da == xatom[XembedInfo] && dl == 2)
            atom = ((Atom *)p)[1];
        XFree(p);
    }
    return atom;
}

int getrootptr(int *x, int *y) {
    int di;
    unsigned int dui;
    Window dummy;

    return XQueryPointer(dpy, root, &dummy, &dummy, x, y, &di, &di, &dui);
}

Client *getcursorclient() {
    int di;
    int dum;
    unsigned int dui;
    Window dummy;
    Window returnwin;

    XQueryPointer(dpy, root, &dummy, &returnwin, &dum, &dum, &di, &di, &dui);
    if (returnwin == root)
        return NULL;
    else
        return wintoclient(returnwin);
}

long getstate(Window w) {
    int format;
    long result = -1;
    unsigned char *p = NULL;
    unsigned long n, extra;
    Atom real;

    if (XGetWindowProperty(dpy, w, wmatom[WMState], 0L, 2L, False,
                           wmatom[WMState], &real, &format, &n, &extra,
                           (unsigned char **)&p) != Success)
        return -1;
    if (n != 0)
        result = *p;
    XFree(p);
    return result;
}

unsigned int getsystraywidth() {
    unsigned int w = 0;
    Client *i;
    if (showsystray)
        for (i = systray->icons; i; w += i->w + systrayspacing, i = i->next)
            ;
    return w ? w + systrayspacing : 1;
}

int gettextprop(Window w, Atom atom, char *text, unsigned int size) {
    char **list = NULL;
    int n;
    XTextProperty name;

    if (!text || size == 0)
        return 0;
    text[0] = '\0';
    if (!XGetTextProperty(dpy, w, &name, atom) || !name.nitems)
        return 0;
    if (name.encoding == XA_STRING)
        strncpy(text, (char *)name.value, size - 1);
    else {
        if (XmbTextPropertyToTextList(dpy, &name, &list, &n) >= Success &&
            n > 0 && *list) {
            strncpy(text, *list, size - 1);
            XFreeStringList(list);
        }
    }
    text[size - 1] = '\0';
    XFree(name.value);
    return 1;
}

void grabbuttons(Client *c, int focused) {
    updatenumlockmask();
    {
        unsigned int i, j;
        unsigned int modifiers[] = {0, LockMask, numlockmask,
                                    numlockmask | LockMask};
        XUngrabButton(dpy, AnyButton, AnyModifier, c->win);
        if (!focused)
            XGrabButton(dpy, AnyButton, AnyModifier, c->win, False, BUTTONMASK,
                        GrabModeSync, GrabModeSync, None, None);
        for (i = 0; i < LENGTH(buttons); i++)
            if (buttons[i].click == ClkClientWin)
                for (j = 0; j < LENGTH(modifiers); j++)
                    XGrabButton(dpy, buttons[i].button,
                                buttons[i].mask | modifiers[j], c->win, False,
                                BUTTONMASK, GrabModeAsync, GrabModeSync, None,
                                None);
    }
}

void grabkeys(void) {
    updatenumlockmask();
    {
        unsigned int i, j;
        unsigned int modifiers[] = {0, LockMask, numlockmask,
                                    numlockmask | LockMask};
        KeyCode code;

        XUngrabKey(dpy, AnyKey, AnyModifier, root);
        for (i = 0; i < LENGTH(keys); i++) {
            if ((code = XKeysymToKeycode(dpy, keys[i].keysym)))
                for (j = 0; j < LENGTH(modifiers); j++) {
                    if (freealttab && keys[i].mod == Mod1Mask)
                        continue;
                    XGrabKey(dpy, code, keys[i].mod | modifiers[j], root, True,
                             GrabModeAsync, GrabModeAsync);
                }
        }

        if (!selmon->sel) {
            for (i = 0; i < LENGTH(dkeys); i++) {
                if ((code = XKeysymToKeycode(dpy, dkeys[i].keysym)))
                    for (j = 0; j < LENGTH(modifiers); j++)
                        XGrabKey(dpy, code, dkeys[i].mod | modifiers[j], root,
                                 True, GrabModeAsync, GrabModeAsync);
            }
        }
    }
}

void hide(Client *c) {
    if (!c || HIDDEN(c))
        return;

    int x, y, wi, h;
    x = c->x;
    y = c->y;
    wi = c->w;
    h = c->h;

    if (animated)
        animateclient(c, c->x, bh - c->h + 40, 0, 0, 10, 0);

    Window w = c->win;
    static XWindowAttributes ra, ca;

    // more or less taken directly from blackbox's hide() function
    XGrabServer(dpy);
    XGetWindowAttributes(dpy, root, &ra);
    XGetWindowAttributes(dpy, w, &ca);
    // prevent UnmapNotify events
    XSelectInput(dpy, root, ra.your_event_mask & ~SubstructureNotifyMask);
    XSelectInput(dpy, w, ca.your_event_mask & ~StructureNotifyMask);
    XUnmapWindow(dpy, w);
    setclientstate(c, IconicState);
    XSelectInput(dpy, root, ra.your_event_mask);
    XSelectInput(dpy, w, ca.your_event_mask);
    XUngrabServer(dpy);
    resize(c, x, y, wi, h, 0);

    focus(c->snext);
    arrange(c->mon);
}

void incnmaster(const Arg *arg) {
    int ccount;
    ccount = clientcount();
    if (arg->i > 0) {
        if (selmon->nmaster >= ccount) {
            selmon->nmaster = ccount;
            return;
        }
    }

    selmon->nmaster = selmon->pertag->nmasters[selmon->pertag->curtag] =
        MAX(selmon->nmaster + arg->i, 0);
    arrange(selmon);
}

#ifdef XINERAMA
static int isuniquegeom(XineramaScreenInfo *unique, size_t n,
                        XineramaScreenInfo *info) {
    while (n--)
        if (unique[n].x_org == info->x_org && unique[n].y_org == info->y_org &&
            unique[n].width == info->width && unique[n].height == info->height)
            return 0;
    return 1;
}
#endif /* XINERAMA */

int startswith(const char *a, const char *b) {
    char *checker = NULL;

    checker = strstr(a, b);
    if (checker == a) {
        return 1;
    } else {
        return 0;
    }
}

int xcommand() {
    char command[256];
    char *fcursor; // walks through the command string as we go
    char *indicator = "c;:;";
    int i, argnum;
    Arg arg;

    // Get root name property
    int got_command = gettextprop(root, XA_WM_NAME, command, sizeof(command));
    if (!got_command || !startswith(command, indicator)) {
        return 0; // no command for us passed, get out
    }
    fcursor =
        command + strlen(indicator); // got command for us, strip indicator

    // Check if a command was found, and if so handle it
    for (i = 0; i < LENGTH(commands); i++) {
        if (!startswith(fcursor, commands[i].cmd))
            continue;

        fcursor += strlen(commands[i].cmd);
        // no args
        if (!strlen(fcursor)) {
            arg = commands[i].arg;
        } else {
            if (fcursor[0] != ';') {
                // longer command staring with the same letters?
                fcursor -= strlen(commands[i].cmd);
                continue;
            }
            fcursor++;
            switch (commands[i].type) {
            case 0: // command without argument
                arg = commands[i].arg;
                break;
            case 1: // toggle-type argument
                argnum = atoi(fcursor);
                if (argnum != 0 && fcursor[0] != '0') {
                    arg = ((Arg){.ui = atoi(fcursor)});
                } else {
                    arg = commands[i].arg;
                }
                break;
            case 3: // tag-type argument (bitmask)
                argnum = atoi(fcursor);
                if (argnum != 0 && fcursor[0] != '0') {
                    arg = ((Arg){.ui = (1 << (atoi(fcursor) - 1))});
                } else {
                    arg = commands[i].arg;
                }
                break;
            case 4: // string argument
                arg = ((Arg){.v = fcursor});
                break;
            case 5: // integer argument
                if (fcursor[0] != '\0') {
                    arg = ((Arg){.i = atoi(fcursor)});
                } else {
                    arg = commands[i].arg;
                }
                break;
            }
        }
        commands[i].func(&(arg));
        break;
    }
    return 1;
}

void keypress(XEvent *e) {

    unsigned int i;
    KeySym keysym;
    XKeyEvent *ev;

    ev = &e->xkey;
    keysym = XKeycodeToKeysym(dpy, (KeyCode)ev->keycode, 0);
    for (i = 0; i < LENGTH(keys); i++) {
        if (keysym == keys[i].keysym &&
            CLEANMASK(keys[i].mod) == CLEANMASK(ev->state) && keys[i].func) {
            keys[i].func(&(keys[i].arg));
        }
    }

    if (!selmon->sel) {
        for (i = 0; i < LENGTH(dkeys); i++) {
            if (keysym == dkeys[i].keysym &&
                CLEANMASK(dkeys[i].mod) == CLEANMASK(ev->state) &&
                dkeys[i].func)
                dkeys[i].func(&(dkeys[i].arg));
        }
    }
}

// close selected client
void killclient(const Arg *arg) {
    if (!selmon->sel || selmon->sel->islocked)
        return;
    if (animated && selmon->sel != animclient && !selmon->sel->isfullscreen) {
        animclient = selmon->sel;
        animateclient(selmon->sel, selmon->sel->x, selmon->mh - 20, 0, 0, 10,
                      0);
    }
    if (!sendevent(selmon->sel->win, wmatom[WMDelete], NoEventMask,
                   wmatom[WMDelete], CurrentTime, 0, 0, 0)) {
        XGrabServer(dpy);
        XSetErrorHandler(xerrordummy);
        XSetCloseDownMode(dpy, DestroyAll);
        XKillClient(dpy, selmon->sel->win);
        XSync(dpy, False);
        XSetErrorHandler(xerror);
        XUngrabServer(dpy);
    }
}

// close client from arg->v
void closewin(const Arg *arg) {
    Client *c = (Client *)arg->v;

    if (!c || c->islocked)
        return;

    animateclient(c, c->x, selmon->mh - 20, 0, 0, 10, 0);

    if (!sendevent(c->win, wmatom[WMDelete], NoEventMask, wmatom[WMDelete],
                   CurrentTime, 0, 0, 0)) {
        XGrabServer(dpy);
        XSetErrorHandler(xerrordummy);
        XSetCloseDownMode(dpy, DestroyAll);
        XKillClient(dpy, c->win);
        XSync(dpy, False);
        XSetErrorHandler(xerror);
        XUngrabServer(dpy);
    }
}

void manage(Window w, XWindowAttributes *wa) {

    if (desktopicons) {
        int x, y;
        Monitor *tempmon;
        if (getrootptr(&x, &y)) {
            tempmon = recttomon(x, y, 1, 1);
            if (selmon != tempmon) {
                if (selmon->sel)
                    unfocus(selmon->sel, 1);
                selmon = tempmon;
                focus(NULL);
            }
        }
    }

    Client *c, *t = NULL;
    Window trans = None;
    XWindowChanges wc;

    c = ecalloc(1, sizeof(Client));
    c->win = w;
    /* geometry */
    c->x = c->oldx = wa->x;
    c->y = c->oldy = wa->y;
    c->w = c->oldw = wa->width;
    c->h = c->oldh = wa->height;
    c->oldbw = wa->border_width;

    updatetitle(c);
    if (XGetTransientForHint(dpy, w, &trans) && (t = wintoclient(trans))) {
        c->mon = t->mon;
        c->tags = t->tags;
    } else {
        c->mon = selmon;
        applyrules(c);
    }

    if (c->x + WIDTH(c) > c->mon->mx + c->mon->mw)
        c->x = c->mon->mx + c->mon->mw - WIDTH(c);
    if (c->y + HEIGHT(c) > c->mon->my + c->mon->mh)
        c->y = c->mon->my + c->mon->mh - HEIGHT(c);
    c->x = MAX(c->x, c->mon->mx);
    /* only fix client y-offset, if the client center might cover the bar */
    c->y = MAX(c->y, ((c->mon->by == c->mon->my) &&
                      (c->x + (c->w / 2) >= c->mon->wx) &&
                      (c->x + (c->w / 2) < c->mon->wx + c->mon->ww))
                         ? bh
                         : c->mon->my);
    c->bw = borderpx;

    if (!c->isfloating && &monocle == c->mon->lt[c->mon->sellt]->arrange &&
        c->w > c->mon->mw - 30 && c->h > (c->mon->mh - 30 - bh)) {
        wc.border_width = 0;
    } else {
        wc.border_width = c->bw;
    }

    XConfigureWindow(dpy, w, CWBorderWidth, &wc);
    XSetWindowBorder(dpy, w, borderscheme[SchemeBorderNormal].pixel);
    configure(c); /* propagates border_width, if size doesn't change */
    updatewindowtype(c);
    updatesizehints(c);
    updatewmhints(c);

    {
        int format;
        unsigned long *data, n, extra;
        Monitor *m;
        Atom atom;
        if (XGetWindowProperty(dpy, c->win, netatom[NetClientInfo], 0L, 2L, False, XA_CARDINAL,
                &atom, &format, &n, &extra, (unsigned char **)&data) == Success && n == 2) {
            c->tags = *data;
            for (m = mons; m; m = m->next) {
                if (m->num == *(data+1)) {
                    c->mon = m;
                    break;
                }
            }
        }
        if (n > 0)
            XFree(data);
    }
    setclienttagprop(c);

    updatemotifhints(c);

    c->sfx = c->x;
    c->sfy = c->y = c->y >= c->mon->my ? c->y : c->y + c->mon->my;
    c->sfw = c->w;
    c->sfh = c->h;
    XSelectInput(dpy, w,
                 EnterWindowMask | FocusChangeMask | PropertyChangeMask |
                     StructureNotifyMask);
    grabbuttons(c, 0);
    if (!c->isfloating)
        c->isfloating = c->oldstate = trans != None || c->isfixed;
    if (c->isfloating)
        XRaiseWindow(dpy, c->win);
    attach(c);
    attachstack(c);
    XChangeProperty(dpy, root, netatom[NetClientList], XA_WINDOW, 32,
                    PropModeAppend, (unsigned char *)&(c->win), 1);
    XMoveResizeWindow(dpy, c->win, c->x + 2 * sw, c->y, c->w,
                      c->h); /* some windows require this */
    if (!HIDDEN(c))
        setclientstate(c, NormalState);
    if (c->mon == selmon)
        unfocus(selmon->sel, 0);
    c->mon->sel = c;
    arrange(c->mon);
    if (!HIDDEN(c))
        XMapWindow(dpy, c->win);
    focus(NULL);
    if (newdesktop) {
        newdesktop = 0;
        createdesktop();
    }

    if (animated && !c->isfullscreen) {
        resizeclient(c, c->x, c->y - 70, c->w, c->h);
        animateclient(c, c->x, c->y + 70, 0, 0, 7, 0);
        if (NULL == c->mon->lt[selmon->sellt]->arrange) {
            XRaiseWindow(dpy, c->win);
        } else {
            if (c->w > selmon->mw - 30 || c->h > selmon->mh - 30)
                arrange(selmon);
        }
    }
}

void mappingnotify(XEvent *e) {
    XMappingEvent *ev = &e->xmapping;

    XRefreshKeyboardMapping(ev);
    if (ev->request == MappingKeyboard)
        grabkeys();
}

void maprequest(XEvent *e) {
    static XWindowAttributes wa;
    XMapRequestEvent *ev = &e->xmaprequest;
    Client *i;
    if ((i = wintosystrayicon(ev->window))) {
        sendevent(i->win, netatom[Xembed], StructureNotifyMask, CurrentTime,
                  XEMBED_WINDOW_ACTIVATE, 0, systray->win,
                  XEMBED_EMBEDDED_VERSION);
        resizebarwin(selmon);
        updatesystray();
    }

    if (!XGetWindowAttributes(dpy, ev->window, &wa))
        return;
    if (wa.override_redirect)
        return;
    if (!wintoclient(ev->window))
        manage(ev->window, &wa);
}

// gets triggered on mouse movement
void motionnotify(XEvent *e) {
    Monitor *m;
    Client *c;
    XMotionEvent *ev = &e->xmotion;
    int x;
    int i;

    if (ev->window != root)
        return;

    if (!tagwidth)
        tagwidth = gettagwidth();

    // detect mouse hovering over other monitor
    m = recttomon(ev->x_root, ev->y_root, 1, 1);
    if (m && m != selmon && focusfollowsmouse) {
        unfocus(selmon->sel, 1);
        selmon = m;
        focus(NULL);
        return;
    }

    // hover below bar
    // TODO: fix bar on bottom
    // leave small deactivator zone
    if (ev->y_root >= selmon->my + bh - 3) {
        // hover near floating sel to resize, don't do it if desktop is covered
        if (selmon->sel && (selmon->sel->isfloating ||
                            NULL == selmon->lt[selmon->sellt]->arrange)) {
            Client *c;
            int tilefound = 0;
            for (c = m->clients; c; c = c->next) {
                if (ISVISIBLE(c) &&
                    !(c->isfloating ||
                      NULL == selmon->lt[selmon->sellt]->arrange)) {
                    tilefound = 1;
                    break;
                }
            }
            if (!tilefound) {
                resizeborder(NULL);
                Client *newc = getcursorclient();
                if (newc && newc != selmon->sel)
                    focus(newc);
            }
        }
        // hover over right side of desktop for slider
        if (ev->x_root > selmon->mx + selmon->mw - 50) {
            if (!altcursor && ev->y_root > bh + 60) {
                altcursor = 2;
                XDefineCursor(dpy, root, cursor[CurVert]->cursor);
            }
            return;
        } else if (altcursor == 2 || altcursor == 1) {
            altcursor = 0;
            XUndefineCursor(dpy, root);
            XDefineCursor(dpy, root, cursor[CurNormal]->cursor);
            return;
        }
        resetbar();
        // return to default cursor after resize hover
        if (altcursor == 2) {
            resetcursor();
        }
        return;
    } else {
        barleavestatus = 1;
    }

    // toggle overlay gesture
    if (ev->y_root == selmon->my &&
        ev->x_root >= selmon->mx + selmon->ww - 20 - getsystraywidth()) {
        if (selmon->gesture != 11) {
            selmon->gesture = 11;
            setoverlay();
        }
        return;
    } else if (selmon->gesture == 11 &&
               ev->x_root >= selmon->mx + selmon->ww - 24 - getsystraywidth()) {
        selmon->gesture = 0;
        return;
    }

    // cursor is to the left of window titles
    if (ev->x_root < selmon->mx + tagwidth + 60) {
        if (selmon->hoverclient)
            selmon->hoverclient = NULL;

        // don't animate if vacant tags are hidden
        if (ev->x_root < selmon->mx + tagwidth && !selmon->showtags) {
            // hover over start menu
            if (ev->x_root < selmon->mx + startmenusize) {
                selmon->gesture = 13;
                drawbar(selmon);
            } else {
                // hover over tag indicators
                i = 0;
                int x = selmon->mx + startmenusize;
                do {
                    x += TEXTW(tags[i]);
                } while (ev->x_root >= x && ++i < 8);

                if (i != selmon->gesture - 1) {
                    selmon->gesture = i + 1;
                    drawbar(selmon);
                }
            }
        } else
            resetbar();
    } else if (selmon->sel &&
               ev->x_root < selmon->mx + 60 + tagwidth + selmon->btw) {
        // cursor is on window titles

        // hover over close button
        if (ev->x_root > selmon->activeoffset &&
            ev->x_root < (selmon->activeoffset + 32)) {
            if (selmon->gesture != 12) {
                selmon->gesture = 12;
                drawbar(selmon);
            }
        } else if (selmon->gesture == 12) {
            selmon->gesture = 0;
            drawbar(selmon);
        } else {
            // hover over resize widget
            if (!altcursor) {
                if (ev->x_root > selmon->activeoffset +
                                     (1.0 / (double)selmon->bt) * selmon->btw -
                                     30 &&
                    ev->x_root < selmon->activeoffset +
                                     (1.0 / (double)selmon->bt) * selmon->btw) {
                    XDefineCursor(dpy, root, cursor[CurResize]->cursor);
                    altcursor = 1;
                }
            } else if (ev->x_root <
                           selmon->activeoffset +
                               (1.0 / (double)selmon->bt) * selmon->btw - 30 ||
                       ev->x_root >
                           selmon->activeoffset +
                               (1.0 / (double)selmon->bt) * selmon->btw) {
                XDefineCursor(dpy, root, cursor[CurNormal]->cursor);
                altcursor = 0;
            }
        }

        // indicator when hovering over clients
        if (selmon->stack) {
            x = selmon->mx + tagwidth + 60;
            c = selmon->clients;
            do {
                if (!ISVISIBLE(c))
                    continue;
                else
                    x += (1.0 / (double)selmon->bt) * selmon->btw;
            } while (ev->x_root > x && (c = c->next));

            if (c) {
                if (c != selmon->hoverclient) {
                    selmon->hoverclient = c;
                    selmon->gesture = 0;
                    drawbar(selmon);
                }
            }
        }
    } else {
        resetbar();
    }
}

void resetbar() {
    if (!selmon->hoverclient && !selmon->gesture)
        return;
    selmon->hoverclient = NULL;
    selmon->gesture = 0;
    if (altcursor)
        resetcursor();
    drawbar(selmon);
}

// drag a window around using the mouse
void movemouse(const Arg *arg) {
    int x, y, ocx, ocy, nx, ny, ti, tx, occ, colorclient, tagx, notfloating;
    Client *c;
    Monitor *m;
    XEvent ev;
    Time lasttime = 0;
    notfloating = 0;
    occ = 0;
    tagx = 0;
    colorclient = 0;

    // some windows are immovable
    if (!(c = selmon->sel) || (c->isfullscreen && !c->isfakefullscreen) ||
        c == selmon->overlay)
        return;

    if (c == selmon->fullscreen) {
        tempfullscreen();
        return;
    }

    if (c->snapstatus) {
        resetsnap(c);
        return;
    }

    if (NULL == selmon->lt[selmon->sellt]->arrange) {
        // unmaximize in floating layout
        if (c->x >= selmon->mx - 100 && c->y >= selmon->my + bh - 100 &&
            c->w >= selmon->mw - 100 && c->h >= selmon->mh - 100) {
            resize(c, c->sfx, c->sfy, c->sfw, c->sfh, 0);
        }
    }

    restack(selmon);
    ocx = c->x;
    ocy = c->y;
    // make pointer grabby shape
    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurMove]->cursor, CurrentTime) != GrabSuccess)
        return;
    if (!getrootptr(&x, &y))
        return;
    bardragging = 1;
    do {
        XMaskEvent(dpy, MOUSEMASK | ExposureMask | SubstructureRedirectMask,
                   &ev);
        switch (ev.type) {
        case ConfigureRequest:
        case Expose:
        case MapRequest:
            handler[ev.type](&ev);
            break;
        case MotionNotify:
            if ((ev.xmotion.time - lasttime) <=
                (1000 / (doubledraw ? 240 : 120)))
                continue;
            lasttime = ev.xmotion.time;

            nx = ocx + (ev.xmotion.x - x);
            // check if client is in snapping range
            if (ev.xmotion.y_root > (selmon->my + (selmon->showbar ? bh : 5))) {
                ny = ocy + (ev.xmotion.y - y);
                if ((ev.xmotion.x_root < selmon->mx + 50 &&
                     ev.xmotion.x_root > selmon->mx - 1) ||
                    (ev.xmotion.x_root > selmon->mx + selmon->mw - 50 &&
                     ev.xmotion.x_root < selmon->mx + selmon->mw)) {
                    if (!colorclient) {
                        XSetWindowBorder(dpy, selmon->sel->win,
                                         borderscheme[SchemeBorderSnap].pixel);
                        colorclient = 1;
                    }
                } else if (colorclient) {
                    colorclient = 0;
                    XSetWindowBorder(
                        dpy, selmon->sel->win,
                        borderscheme[SchemeBorderFloatFocus].pixel);
                }
            } else {
                ny = selmon->my + (selmon->showbar ? bh : 0);
                if (!colorclient) {
                    colorclient = 1;
                    XSetWindowBorder(dpy, selmon->sel->win,
                                     borderscheme[SchemeBorderSnap].pixel);
                }
            }

            if (abs(selmon->wx - nx) < snap)
                nx = selmon->wx;
            else if (abs((selmon->wx + selmon->ww) - (nx + WIDTH(c))) < snap)
                nx = selmon->wx + selmon->ww - WIDTH(c);
            if (abs(selmon->wy - ny) < snap)
                ny = selmon->wy;
            else if (abs((selmon->wy + selmon->wh) - (ny + HEIGHT(c))) < snap)
                ny = selmon->wy + selmon->wh - HEIGHT(c);
            if (!c->isfloating && selmon->lt[selmon->sellt]->arrange &&
                (abs(nx - c->x) > snap || abs(ny - c->y) > snap)) {
                int tmpanimated;
                float cursoraspectx, cursoraspecty;
                if (animated) {
                    animated = 0;
                    tmpanimated = 1;
                } else {
                    tmpanimated = 0;
                }
                // cursor is within window dimensions
                if (ev.xmotion.x_root > c->x &&
                    ev.xmotion.x_root < c->x + c->w &&
                    ev.xmotion.y_root > c->y &&
                    ev.xmotion.y_root < c->y + c->h) {
                    cursoraspectx =
                        ((float)(ev.xmotion.x_root - ocx + 1) / c->w);
                    cursoraspecty =
                        ((float)(ev.xmotion.y_root - ocy + 1) / c->h);
                    c->w = c->sfw;
                    c->h = c->sfh;
                    c->x = ev.xmotion.x_root - c->w * cursoraspectx;
                    c->y = ev.xmotion.y_root - c->h * cursoraspecty;
                    nx = c->x;
                    ny = c->y;
                    ocy = c->y;
                    ocx = c->x;
                    savefloating(c);
                }
                togglefloating(NULL);
                if (tmpanimated) {
                    animated = 1;
                }
            }

            if (!selmon->lt[selmon->sellt]->arrange || c->isfloating)
                resize(c, nx, ny, c->w, c->h, 1);

            if (ev.xmotion.x_root < selmon->mx ||
                ev.xmotion.x_root > selmon->mx + selmon->mw ||
                ev.xmotion.y_root < selmon->my ||
                ev.xmotion.y_root > selmon->my + selmon->mh) {
                if ((m = recttomon(ev.xmotion.x_root, ev.xmotion.y_root, 2,
                                   2)) != selmon) {
                    pausedraw = 1;
                    XRaiseWindow(dpy, c->win);
                    sendmon(c, m);
                    unfocus(selmon->sel, 0);
                    selmon = m;
                    focus(NULL);
                    pausedraw = 0;
                    drawbars();
                }
            }
            if (ev.xmotion.y_root < selmon->my + bh &&
                tagx != getxtag(ev.xmotion.x_root)) {
                tagx = getxtag(ev.xmotion.x_root);
                selmon->gesture = tagx + 1;
                drawbar(selmon);
            }

            break;
        }
    } while (ev.type != ButtonRelease);

    // user let go of the window
    bardragging = 0;
    // dragging on top of the bar
    if (ev.xmotion.y_root < selmon->my + bh &&
        ev.xmotion.y_root > selmon->my - 1) {
        if (!tagwidth)
            tagwidth = gettagwidth();

        // drag on top of tag area
        if (ev.xmotion.x_root < selmon->mx + tagwidth &&
            ev.xmotion.x_root > selmon->mx) {
            ti = 0;
            tx = startmenusize;
            m = selmon;
            for (c = selmon->clients; c; c = c->next)
                occ |= c->tags == 255 ? 0 : c->tags;
            do {
                // do not reserve space for vacant tags
                if (ti >= 9)
                    continue;
                if (selmon->showtags) {
                    if (!(occ & 1 << ti || m->tagset[m->seltags] & 1 << ti))
                        continue;
                }
                tx += TEXTW(tags[ti]);
            } while (ev.xmotion.x_root >= tx + selmon->mx &&
                     ++ti < LENGTH(tags));
            selmon->sel->isfloating = 0;
            if (ev.xmotion.state & ShiftMask)
                tag(&((Arg){.ui = 1 << ti}));
            else
                followtag(&((Arg){.ui = 1 << ti}));

        } else if (ev.xmotion.x_root > selmon->mx + selmon->mw - 50 &&
                   ev.xmotion.x_root < selmon->mx + selmon->mw) {
            // drag on top right corner
            resize(selmon->sel, selmon->mx + 20, bh, selmon->ww - 40,
                   (selmon->mh) / 3, True);
            togglefloating(NULL);
            createoverlay();
            selmon->gesture = 11;
        } else if (selmon->sel->isfloating ||
                   NULL == selmon->lt[selmon->sellt]->arrange) {
            // drag on top of window area
            notfloating = 1;
        }
    } else {
        if (ev.xmotion.x_root > selmon->mx + selmon->mw - 50 &&
            ev.xmotion.x_root < selmon->mx + selmon->mw + 1) {
            // snap to half of the screen like on gnome, right side
            if (ev.xmotion.state & ShiftMask ||
                NULL == c->mon->lt[c->mon->sellt]->arrange) {
                XSetWindowBorder(dpy, selmon->sel->win,
                                 borderscheme[SchemeBorderTileFocus].pixel);

                c->sfh = c->h;
                c->sfw = c->w;
                c->sfx = ocx;
                c->sfy = ocy;

                if (ev.xmotion.y_root < selmon->my + selmon->mh / 7)
                    c->snapstatus = 2;
                else if (ev.xmotion.y_root > selmon->my + 6 * (selmon->mh / 7))
                    c->snapstatus = 4;
                else
                    c->snapstatus = 3;
                applysnap(c, c->mon);
            } else {
                if (ev.xmotion.y_root < selmon->my + (2 * selmon->mh) / 3)
                    moveright(arg);
                else
                    tagtoright(arg);
                c->isfloating = 0;
                arrange(selmon);
            }

        } else if (ev.xmotion.x_root < selmon->mx + 50 &&
                   ev.xmotion.x_root > selmon->mx - 1) {
            // snap to half of the screen like on gnome, left side
            if (ev.xmotion.state & ShiftMask ||
                NULL == c->mon->lt[c->mon->sellt]->arrange) {
                XSetWindowBorder(dpy, selmon->sel->win,
                                 borderscheme[SchemeBorderTileFocus].pixel);

                c->sfh = c->h;
                c->sfw = c->w;
                c->sfx = ocx;
                c->sfy = ocy;

                if (ev.xmotion.y_root < selmon->my + selmon->mh / 7)
                    c->snapstatus = 8;
                else if (ev.xmotion.y_root > selmon->my + 6 * (selmon->mh / 7))
                    c->snapstatus = 6;
                else
                    c->snapstatus = 7;
                applysnap(c, c->mon);
            } else {
                if (ev.xmotion.y_root < selmon->my + (2 * selmon->mh) / 3)
                    moveleft(arg);
                else
                    tagtoleft(arg);
                c->isfloating = 0;
                arrange(selmon);
            }
        }
    }

    XUngrabPointer(dpy, CurrentTime);

    if (notfloating) {
        if (NULL != selmon->lt[selmon->sellt]->arrange) {
            togglefloating(NULL);
        } else {
            // maximize window
            XSetWindowBorder(dpy, selmon->sel->win,
                             borderscheme[SchemeBorderTileFocus].pixel);
            savefloating(c);
            selmon->sel->snapstatus = 9;
            arrange(selmon);
        }
    }
}

// drag up and down on the desktop to
// change volume or start onboard by dragging to the right
void gesturemouse(const Arg *arg) {
    int x, y, lasty;
    XEvent ev;
    Time lasttime = 0;
    int tmpactive = 0;
    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurMove]->cursor, CurrentTime) != GrabSuccess)
        return;
    if (!getrootptr(&x, &y))
        return;
    lasty = y;
    do {
        XMaskEvent(dpy, MOUSEMASK | ExposureMask | SubstructureRedirectMask,
                   &ev);
        switch (ev.type) {
        case ConfigureRequest:
        case Expose:
        case MapRequest:
            handler[ev.type](&ev);
            break;
        case MotionNotify:
            if ((ev.xmotion.time - lasttime) <=
                (1000 / (doubledraw ? 240 : 120)))
                continue;
            lasttime = ev.xmotion.time;
            if (abs(lasty - ev.xmotion.y_root) > selmon->mh / 30) {
                if (ev.xmotion.y_root < lasty)
                    spawn(&((Arg){.v = upvol}));
                else
                    spawn(&((Arg){.v = downvol}));
                lasty = ev.xmotion.y_root;
                if (!tmpactive)
                    tmpactive = 1;
            }
            break;
        }
    } while (ev.type != ButtonRelease);

    if (ev.xmotion.x_root < selmon->mx + selmon->mw - 100) {
        spawn(&((Arg){.v = onboardcmd}));
    } else {
        if (!tmpactive && abs(ev.xmotion.y_root - y) < 100) {
            spawn(&((Arg){.v = caretinstantswitchcmd}));
        }
    }

    XUngrabPointer(dpy, CurrentTime);
}

// hover over the border to move/resize a window
int resizeborder(const Arg *arg) {
    if (!(selmon->sel &&
          (selmon->sel->isfloating || !selmon->lt[selmon->sellt]->arrange)))
        return 0;
    XEvent ev;
    Time lasttime = 0;
    Client *c;
    int inborder = 1;
    int x, y;
    getrootptr(&x, &y);
    c = selmon->sel;

    if ((selmon->showbar && y < selmon->my + bh) ||
        (y > c->y && y < c->y + c->h && x > c->x && x < c->x + c->w) ||
        y < c->y - 30 || x < c->x - 30 || y > c->y + c->h + 30 ||
        x > c->x + c->w + 30) {
        return 1;
    }

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurResize]->cursor,
                     CurrentTime) != GrabSuccess)
        return 0;

    do {
        XMaskEvent(dpy,
                   MOUSEMASK | ExposureMask | KeyPressMask |
                       SubstructureRedirectMask,
                   &ev);
        switch (ev.type) {
        case ConfigureRequest:
        case Expose:
        case MapRequest:
            handler[ev.type](&ev);
            break;
        case KeyPress:
            handler[ev.type](&ev);
            break;
        case MotionNotify:
            x = ev.xmotion.x_root;
            y = ev.xmotion.y_root;

            if ((ev.xmotion.time - lasttime) <= (1000 / 60))
                continue;
            lasttime = ev.xmotion.time;
            if ((y > c->y && y < c->y + c->h && x > c->x && x < c->x + c->w) ||
                (selmon->showbar && y < selmon->my + bh)) {
                XUngrabPointer(dpy, CurrentTime);
                return 0;
            }
            if (y < c->y - 30 || x < c->x - 30 || y > c->y + c->h + 30 ||
                x > c->x + c->w + 30)
                inborder = 0;
        }
    } while (ev.type != ButtonPress && inborder);
    XUngrabPointer(dpy, CurrentTime);
    if (ev.type == ButtonPress) {
        resetsnap(c);
        switch (ev.xbutton.button) {
        case Button3:
            warpinto(c);
            movemouse(NULL);
            break;
        case Button2:
            killclient(NULL);
            break;
        default:
            if (y < c->y && x > c->x + (c->w * 0.5) - c->w / 4 &&
                x < c->x + (c->w * 0.5) + c->w / 4) {
                XWarpPointer(dpy, None, root, 0, 0, 0, 0, x, c->y + 10);
                movemouse(NULL);
            } else {
                resizemouse(NULL);
            }
            break;
        }
        return 0;
    } else {
        return 1;
    }
}

// drag clients around the top bar
void dragmouse(const Arg *arg) {
    int x, y, starty, startx, dragging, tabdragging, isactive, sinit;
    starty = 100;
    sinit = 0;
    dragging = 0;
    tabdragging = 0;
    XEvent ev;
    Time lasttime = 0;

    Client *tempc = (Client *)arg->v;
    resetbar();
    if (tempc->isfullscreen &&
        !tempc->isfakefullscreen) /* no support moving fullscreen windows by
                                     mouse */
        return;
    if (!getrootptr(&x, &y))
        return;
    if (x > selmon->activeoffset + (1.0 / (double)selmon->bt) * selmon->btw -
                30 &&
        x < selmon->activeoffset + (1.0 / (double)selmon->bt) * selmon->btw) {
        drawwindow(NULL);
        return;
    }

    if (tempc == selmon->overlay) {
        setoverlay();
        return;
    }

    if (tempc != selmon->sel) {
        if (HIDDEN(tempc)) {
            show(tempc);
            focus(tempc);
            restack(selmon);
            return;
        }
        isactive = 0;
        focus(tempc);
        restack(selmon);
        if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync,
                         GrabModeAsync, None, cursor[CurClick]->cursor,
                         CurrentTime) != GrabSuccess)
            return;

    } else {
        if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync,
                         GrabModeAsync, None, cursor[CurMove]->cursor,
                         CurrentTime) != GrabSuccess)
            return;
        isactive = 1;
    }

    Client *c = selmon->sel;

    do {
        XMaskEvent(dpy, MOUSEMASK | ExposureMask | SubstructureRedirectMask,
                   &ev);
        switch (ev.type) {
        case ConfigureRequest:
        case Expose:
        case MapRequest:
            handler[ev.type](&ev);
            break;
        case MotionNotify:
            if ((ev.xmotion.time - lasttime) <= (1000 / 60))
                continue;
            lasttime = ev.xmotion.time;

            if (!sinit) {
                starty = ev.xmotion.y_root;
                startx = ev.xmotion.x_root;
                sinit = 1;
            } else {
                if ((abs((starty - ev.xmotion.y_root) *
                         (starty - ev.xmotion.y_root)) +
                     abs((startx - ev.xmotion.x_root) *
                         (startx - ev.xmotion.x_root))) > 4069) {
                    dragging = 1;
                    if (ev.xmotion.y_root < selmon->wy) {
                        tabdragging = 1;
                    }
                }
                if (starty > 10 && ev.xmotion.y_root == 0 && c->isfloating)
                    dragging = 1;
            }
        }
    } while (ev.type != ButtonRelease && !dragging);

    if (tabdragging) {
        int prev_slot = -1;
        int tempanim = animated;
        animated = 0;
        drawbar(c->mon);
        do {
            XMaskEvent(dpy, MOUSEMASK | ExposureMask | SubstructureRedirectMask,
                       &ev);
            switch (ev.type) {
            case ConfigureRequest:
            case Expose:
            case MapRequest:
                handler[ev.type](&ev);
                break;
            case MotionNotify:
                if ((ev.xmotion.time - lasttime) <= (1000 / 60))
                    continue;
                lasttime = ev.xmotion.time;
                if (ev.xmotion.y_root >= selmon->wy) {
                    tabdragging = 0;
                    break;
                }
                int x = ev.xmotion.x_root;
                // startmenu + tags + layout indicator
                int left = selmon->mx + startmenusize + tagwidth + bh;
                int right = left + selmon->btw;
                if (x < left || x >= right) {
                    tabdragging = 0;
                    break;
                }
                int slot = (x - left) * selmon->bt / selmon->btw;
                if (slot != prev_slot) {
                    prev_slot = slot;
                    detach(c);
                    // walk down linked list to the slot #
                    Client **tc = &selmon->clients;
                    int i = 0;
                    while (i < slot && *tc) {
                        if (*tc && ISVISIBLE((*tc)))
                            i++;
                        tc = &(*tc)->next;
                    }
                    c->next = *tc;
                    *tc = c;
                    arrange(selmon);
                }
                break;
            case ButtonRelease:
                dragging = 0;
            }
        } while (dragging && tabdragging);
        animated = tempanim;
    }

    if (dragging) {
        if (!c->isfloating) {
            c->sfy = selmon->my + bh;
            if (animated) {
                animateclient(selmon->sel, selmon->sel->sfx, selmon->sel->sfy,
                              selmon->sel->sfw, selmon->sel->sfh, 5, 0);
                animated = 0;
                togglefloating(NULL);
                animated = 1;
                getrootptr(&x, &y);
                if (y > c->y + 20)
                    resize(c, c->x, y - 20, c->w, c->h, 1);
            } else {
                togglefloating(NULL);
            }
        }
        resetsnap(c);
        if (ev.xmotion.x_root > c->x && ev.xmotion.x_root < c->x + c->w)
            XWarpPointer(dpy, None, root, 0, 0, 0, 0, ev.xmotion.x_root,
                         c->y + 20);
        else
            forcewarp(c);
        movemouse(NULL);
    }
    if (!dragging && !tabdragging) {
        if (isactive)
            hide(tempc);
    }

    XUngrabPointer(dpy, CurrentTime);
}

void resetoverlaysize() {
    if (!selmon->overlay)
        return;
    Client *c;
    c = selmon->overlay;
    selmon->overlay->isfloating = 1;
    switch (selmon->overlaymode) {
    case 0:
        resize(c, selmon->mx + 20, bh, selmon->ww - 40, (selmon->wh) / 3, True);
        break;
    case 1:
        resize(c, selmon->mx + selmon->mw - c->w, 40, selmon->mw / 3,
               selmon->mh - 80, True);
        break;
    case 2:
        resize(c, selmon->mx + 20, selmon->my + selmon->mh - c->h,
               selmon->ww - 40, (selmon->wh) / 3, True);
        break;
    case 3:
        resize(c, selmon->mx, 40, selmon->mw / 3, selmon->mh - 80, True);
        break;
    default:
        selmon->overlaymode = 0;
        break;
    }
}

// drag on the top bar with the right mouse
void dragrightmouse(const Arg *arg) {
    int x, y, starty, startx, dragging, sinit;
    starty = 100;
    sinit = 0;
    dragging = 0;
    XEvent ev;
    Time lasttime = 0;

    Client *tempc = (Client *)arg->v;
    resetbar();
    if (tempc->isfullscreen &&
        !tempc->isfakefullscreen) /* no support moving fullscreen windows by
                                     mouse */
        return;

    if (tempc == selmon->overlay) {
        focus(selmon->overlay);
        // reset overlay size
        if (!selmon->overlay->isfloating) {
            changefloating(selmon->overlay);
        }
        resetoverlaysize();
        arrange(selmon);
    }

    Client *c = selmon->sel;

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurResize]->cursor,
                     CurrentTime) != GrabSuccess)
        return;
    if (!getrootptr(&x, &y))
        return;
    do {
        XMaskEvent(dpy, MOUSEMASK | ExposureMask | SubstructureRedirectMask,
                   &ev);
        switch (ev.type) {
        case ConfigureRequest:
        case Expose:
        case MapRequest:
            handler[ev.type](&ev);
            break;
        case MotionNotify:
            if ((ev.xmotion.time - lasttime) <= (1000 / 60))
                continue;
            lasttime = ev.xmotion.time;

            if (!sinit) {
                starty = ev.xmotion.y_root;
                startx = ev.xmotion.x_root;
                sinit = 1;
            } else {
                if ((abs((starty - ev.xmotion.y_root) *
                         (starty - ev.xmotion.y_root)) +
                     abs((startx - ev.xmotion.x_root) *
                         (startx - ev.xmotion.x_root))) > 4069)
                    dragging = 1;
                if (starty > 10 && ev.xmotion.y_root == 0 && c->isfloating)
                    dragging = 1;
            }
            break;
        }
    } while (ev.type != ButtonRelease && !dragging);

    if (dragging) {
        if (tempc != selmon->sel) {
            focus(tempc);
            restack(selmon);
        }
        if (tempc == selmon->overlay) {
            switch (selmon->overlaymode) {
            case 0:
                XWarpPointer(dpy, None, root, 0, 0, 0, 0,
                             tempc->x + (tempc->w / 2), tempc->y + tempc->h);
                break;
            case 1:
                XWarpPointer(dpy, None, root, 0, 0, 0, 0, tempc->x,
                             tempc->y + tempc->h / 2);
                break;
            case 2:
                XWarpPointer(dpy, None, root, 0, 0, 0, 0,
                             tempc->x + (tempc->w / 2), tempc->y);
                break;
            case 3:
                XWarpPointer(dpy, None, root, 0, 0, 0, 0, tempc->x + tempc->h,
                             tempc->y + tempc->h / 2);
                break;
            default:
                break;
            }
        } else {
            XWarpPointer(dpy, None, root, 0, 0, 0, 0, tempc->x + tempc->w,
                         tempc->y + tempc->h);
        }
        if (animated) {
            animated = 0;
            resizemouse(NULL);
            animated = 1;
        } else {
            resizemouse(NULL);
        }

        if (NULL == selmon->lt[selmon->sellt]->arrange) {
            savefloating(c);
        }
    } else {
        if (tempc != selmon->sel) {
            focus(tempc);
        }
        zoom(NULL);
    }

    XUngrabPointer(dpy, CurrentTime);
}

// drag out an area using slop and resize the selected window to it.
void drawwindow(const Arg *arg) {

    char str[100];
    int i;
    char strout[100];
    int dimensions[4];
    int width, height, x, y;
    char tmpstring[30] = {0};
    int firstchar = 0;
    int counter = 0;
    Monitor *m;
    Client *c;

    if (!selmon->sel)
        return;
    FILE *fp = popen("instantslop -f x%xx%yx%wx%hx", "r");

    while (fgets(str, 100, fp) != NULL) {
        strcat(strout, str);
    }

    pclose(fp);

    if (strlen(strout) < 6) {
        return;
    }

    for (i = 0; i < strlen(strout); i++) {
        if (!firstchar) {
            if (strout[i] == 'x') {
                firstchar = 1;
            }
            continue;
        }

        if (strout[i] != 'x') {
            tmpstring[strlen(tmpstring)] = strout[i];
        } else {
            dimensions[counter] = atoi(tmpstring);
            counter++;
            memset(tmpstring, 0, strlen(tmpstring));
        }
    }

    x = dimensions[0];
    y = dimensions[1];
    width = dimensions[2];
    height = dimensions[3];

    if (!selmon->sel)
        return;

    c = selmon->sel;

    if (width > 50 && height > 50 && x > -40 && y > -40 &&
        width < selmon->mw + 40 && height < selmon->mh + 40 &&
        (abs(c->w - width) > 20 || abs(c->h - height) > 20 ||
         abs(c->x - x) > 20 || abs(c->y - y) > 20)) {
        if ((m = recttomon(x, y, width, height)) != selmon) {
            sendmon(c, m);
            unfocus(selmon->sel, 0);
            selmon = m;
            focus(NULL);
        }

        if (!c->isfloating)
            togglefloating(NULL);
        animateclient(c, x, y, width - (c->bw * 2), height - (c->bw * 2), 10,
                      0);
        arrange(selmon);
    } else {
        fprintf(stderr, "errror %s", strout);
    }
    memset(tmpstring, 0, strlen(tmpstring));
}

// drag the green tag mark to another tag
void dragtag(const Arg *arg) {
    if (!tagwidth)
        tagwidth = gettagwidth();
    if ((arg->ui & TAGMASK) != selmon->tagset[selmon->seltags]) {
        view(arg);
        return;
    }

    int x, y, tagx = 0;
    int leftbar = 0;
    XEvent ev;
    Time lasttime = 0;

    if (!selmon->sel)
        return;

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurMove]->cursor, CurrentTime) != GrabSuccess)
        return;
    if (!getrootptr(&x, &y))
        return;
    bardragging = 1;
    do {
        XMaskEvent(dpy, MOUSEMASK | ExposureMask | SubstructureRedirectMask,
                   &ev);
        switch (ev.type) {
        case ConfigureRequest:
        case Expose:
        case MapRequest:
            handler[ev.type](&ev);
            break;
        case MotionNotify:
            if ((ev.xmotion.time - lasttime) <= (1000 / 60))
                continue;
            lasttime = ev.xmotion.time;
            if (ev.xmotion.y_root > selmon->my + bh + 1)
                leftbar = 1;
        }

        if (tagx != getxtag(ev.xmotion.x_root)) {
            tagx = getxtag(ev.xmotion.x_root);
            selmon->gesture = tagx + 1;
            drawbar(selmon);
        }
        // add additional dragging code
    } while (ev.type != ButtonRelease && !leftbar);

    if (!leftbar) {
        if (ev.xmotion.x_root < selmon->mx + tagwidth) {
            if (ev.xmotion.state & ShiftMask) {
                followtag(&((Arg){.ui = 1 << getxtag(ev.xmotion.x_root)}));
            } else if (ev.xmotion.state & ControlMask) {
                tagall(&((Arg){.ui = 1 << getxtag(ev.xmotion.x_root)}));
            } else {
                tag(&((Arg){.ui = 1 << getxtag(ev.xmotion.x_root)}));
            }
        } else if (ev.xmotion.x_root > selmon->mx + selmon->mw - 50) {
            if (selmon->sel == selmon->overlay) {
                setoverlay();
            } else {
                createoverlay();
                selmon->gesture = 11;
            }
        }
    }
    bardragging = 0;
    XUngrabPointer(dpy, CurrentTime);
}

void shutkill(const Arg *arg) {
    if (!selmon->clients)
        spawn(&((Arg){.v = instantshutdowncmd}));
    else
        killclient(arg);
}

void nametag(const Arg *arg) {
    char *p;
    FILE *f;
    int i;

    char *name = (char *)arg->v;

    if (strlen(name) >= MAX_TAGLEN)
        return;

    for (i = 0; i < LENGTH(tags); i++) {
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

Client *nexttiled(Client *c) {
    for (; c && (c->isfloating || !ISVISIBLE(c) || HIDDEN(c)); c = c->next)
        ;
    return c;
}

void pop(Client *c) {
    detach(c);
    attach(c);
    focus(c);
    arrange(c->mon);
}

void propertynotify(XEvent *e) {
    Client *c;
    Window trans;
    XPropertyEvent *ev = &e->xproperty;

    if ((c = wintosystrayicon(ev->window))) {
        if (ev->atom == XA_WM_NORMAL_HINTS) {
            updatesizehints(c);
            updatesystrayicongeom(c, c->w, c->h);
        } else
            updatesystrayiconstate(c, ev);
        resizebarwin(selmon);
        updatesystray();
    }
    if ((ev->window == root) && (ev->atom == XA_WM_NAME)) {
        if (!xcommand())
            updatestatus();
    } else if (ev->state == PropertyDelete)
        return; /* ignore */
    else if ((c = wintoclient(ev->window))) {
        switch (ev->atom) {
        default:
            break;
        case XA_WM_TRANSIENT_FOR:
            if (!c->isfloating && (XGetTransientForHint(dpy, c->win, &trans)) &&
                (c->isfloating = (wintoclient(trans)) != NULL))
                arrange(c->mon);
            break;
        case XA_WM_NORMAL_HINTS:
            c->hintsvalid = 0;
            break;
        case XA_WM_HINTS:
            updatewmhints(c);
            drawbars();
            break;
        }
        if (ev->atom == XA_WM_NAME || ev->atom == netatom[NetWMName]) {
            updatetitle(c);
            if (c == c->mon->sel)
                drawbar(c->mon);
        }
        if (ev->atom == netatom[NetWMWindowType])
            updatewindowtype(c);
        if (ev->atom == motifatom)
            updatemotifhints(c);
    }
}

void quit(const Arg *arg) { running = 0; }

// return monitor that a rectangle is on
Monitor *recttomon(int x, int y, int w, int h) {
    Monitor *m, *r = selmon;
    int a, area = 0;
    for (m = mons; m; m = m->next)
        if ((a = INTERSECT(x, y, w, h, m)) > area) {
            area = a;
            r = m;
        }
    return r;
}

void removesystrayicon(Client *i) {
    Client **ii;

    if (!showsystray || !i)
        return;
    for (ii = &systray->icons; *ii && *ii != i; ii = &(*ii)->next)
        ;
    if (ii)
        *ii = i->next;
    free(i);
}

void resize(Client *c, int x, int y, int w, int h, int interact) {
    if (applysizehints(c, &x, &y, &w, &h, interact) || selmon->clientcount == 1)
        resizeclient(c, x, y, w, h);
}

void resizebarwin(Monitor *m) {
    unsigned int w = m->ww;
    if (showsystray && m == systraytomon(m))
        w -= getsystraywidth();
    XMoveResizeWindow(dpy, m->barwin, m->wx, m->by, w, bh);
}

void resizeclient(Client *c, int x, int y, int w, int h) {
    XWindowChanges wc;

    c->oldx = c->x;
    c->x = wc.x = x;
    c->oldy = c->y;
    c->y = wc.y = y;
    c->oldw = c->w;
    c->w = wc.width = w;
    c->oldh = c->h;
    c->h = wc.height = h;
    wc.border_width = c->bw;

    XConfigureWindow(dpy, c->win,
                     CWX | CWY | CWWidth | CWHeight | CWBorderWidth, &wc);
    configure(c);
    XSync(dpy, False);
}

void forceresizemouse(const Arg *arg) {
    forceresize = 1;
    resizemouse(arg);
    forceresize = 0;
}

// resize a window using the mouse
void resizemouse(const Arg *arg) {
    int ocx, ocy, nw, nh;
    int ocx2, ocy2, nx, ny;
    Client *c;
    Monitor *m;
    XEvent ev;
    Cursor cur;
    int horizcorner, vertcorner;
    int corner;
    int di;
    unsigned int dui;
    Window dummy;
    Time lasttime = 0;

    if (!(c = selmon->sel))
        return;

    if (c == selmon->fullscreen) {
        tempfullscreen();
        return;
    }

    if (c->isfullscreen &&
        !c->isfakefullscreen) /* no support resizing fullscreen windows by mouse
                               */
        return;

    restack(selmon);
    ocx = c->x;
    ocy = c->y;
    ocx2 = c->x + c->w;
    ocy2 = c->y + c->h;

    if (!XQueryPointer(dpy, c->win, &dummy, &dummy, &di, &di, &nx, &ny, &dui))
        return;

    if (ny > c->h / 2) {     // bottom
        if (nx < c->w / 3) { // left
            if (ny < 2 * c->h / 3) {
                corner = 7; // side
                cur = cursor[CurHor]->cursor;
            } else {
                corner = 6; // corner
                cur = cursor[CurBL]->cursor;
            }
        } else if (nx > 2 * c->w / 3) { // right
            if (ny < 2 * c->h / 3) {
                corner = 3; // side
                cur = cursor[CurHor]->cursor;
            } else {
                corner = 4; // corner
                cur = cursor[CurBR]->cursor;
            }
        } else {
            // middle
            corner = 5;
            cur = cursor[CurVert]->cursor;
        }
    } else {                 // top
        if (nx < c->w / 3) { // left
            if (ny > c->h / 3) {
                corner = 7; // side
                cur = cursor[CurHor]->cursor;
            } else {
                corner = 0; // corner
                cur = cursor[CurTL]->cursor;
            }
        } else if (nx > 2 * c->w / 3) { // right
            if (ny > c->h / 3) {
                corner = 3; // side
                cur = cursor[CurHor]->cursor;
            } else {
                corner = 2; // corner
                cur = cursor[CurTR]->cursor;
            }
        } else {
            // cursor on middle
            corner = 1;
            cur = cursor[CurVert]->cursor;
        }
    }

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cur, CurrentTime) != GrabSuccess)
        return;

    horizcorner = nx < c->w / 2;
    vertcorner = ny < c->h / 2;
    if (corner == 0 || corner == 2 || corner == 4 || corner == 6) {
        XWarpPointer(dpy, None, c->win, 0, 0, 0, 0,
                     horizcorner ? (-c->bw) : (c->w + c->bw - 1),
                     vertcorner ? (-c->bw) : (c->h + c->bw - 1));
    } else {
        if (corner == 1 || corner == 5) {
            XWarpPointer(dpy, None, c->win, 0, 0, 0, 0, (c->w + c->bw - 1) / 2,
                         vertcorner ? (-c->bw) : (c->h + c->bw - 1));
        } else if (corner == 3 || corner == 7) {
            XWarpPointer(dpy, None, c->win, 0, 0, 0, 0,
                         horizcorner ? (-c->bw) : (c->w + c->bw - 1),
                         (c->h + c->bw - 1) / 2);
        }
    }

    do {
        XMaskEvent(dpy, MOUSEMASK | ExposureMask | SubstructureRedirectMask,
                   &ev);
        switch (ev.type) {
        case ConfigureRequest:
        case Expose:
        case MapRequest:
            handler[ev.type](&ev);
            break;
        case MotionNotify:
            if ((ev.xmotion.time - lasttime) <=
                (1000 / (doubledraw ? 240 : 120)))
                continue;
            lasttime = ev.xmotion.time;

            if (corner != 1 && corner != 5) {
                nx = horizcorner ? ev.xmotion.x : c->x;
                nw = MAX(horizcorner ? (ocx2 - nx)
                                     : (ev.xmotion.x - ocx - 2 * c->bw + 1),
                         1);
            } else {
                nx = c->x;
                nw = c->w;
            }

            if (corner != 7 && corner != 3) {
                ny = vertcorner ? ev.xmotion.y : c->y;
                nh = MAX(vertcorner ? (ocy2 - ny)
                                    : (ev.xmotion.y - ocy - 2 * c->bw + 1),
                         1);
            } else {
                ny = c->y;
                nh = c->h;
            }

            if (c->mon->wx + nw >= selmon->wx &&
                c->mon->wx + nw <= selmon->wx + selmon->ww &&
                c->mon->wy + nh >= selmon->wy &&
                c->mon->wy + nh <= selmon->wy + selmon->wh) {
                if (!c->isfloating && selmon->lt[selmon->sellt]->arrange &&
                    (abs(nw - c->w) > snap || abs(nh - c->h) > snap)) {
                    if (animated) {
                        animated = 0;
                        togglefloating(NULL);
                        animated = 1;
                    } else {
                        togglefloating(NULL);
                    }
                }
            }
            if (!selmon->lt[selmon->sellt]->arrange || c->isfloating) {
                if (c->bw == 0 && c != selmon->overlay)
                    c->bw = c->oldbw;
                if (!forceresize)
                    resize(c, nx, ny, nw, nh, 1);
                else
                    resizeclient(c, nx, ny, nw, nh);
            }
            break;
        }
    } while (ev.type != ButtonRelease);

    XUngrabPointer(dpy, CurrentTime);
    while (XCheckMaskEvent(dpy, EnterWindowMask, &ev))
        ;
    if ((m = recttomon(c->x, c->y, c->w, c->h)) != selmon) {
        sendmon(c, m);
        unfocus(selmon->sel, 0);
        selmon = m;
        focus(NULL);
    }

    if (NULL == selmon->lt[selmon->sellt]->arrange) {
        savefloating(c);
        c->snapstatus = 0;
    }
}

// resizemouse but keep the aspect ratio
void resizeaspectmouse(const Arg *arg) {
    int ocx, ocy, nw, nh;
    int ocx2, ocy2, nx, ny;
    Client *c;
    Monitor *m;
    XEvent ev;
    int di;
    unsigned int dui;
    Window dummy;
    Time lasttime = 0;

    if (!(c = selmon->sel))
        return;

    if (c->isfullscreen &&
        !c->isfakefullscreen) /* no support resizing fullscreen windows by mouse
                               */
        return;

    restack(selmon);
    ocx = c->x;
    ocy = c->y;
    ocx2 = c->w;
    ocy2 = c->h;
    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurResize]->cursor,
                     CurrentTime) != GrabSuccess)
        return;
    if (!XQueryPointer(dpy, c->win, &dummy, &dummy, &di, &di, &nx, &ny, &dui))
        return;
    XWarpPointer(dpy, None, c->win, 0, 0, 0, 0, c->w + c->bw - 1,
                 c->h + c->bw - 1);

    do {
        XMaskEvent(dpy, MOUSEMASK | ExposureMask | SubstructureRedirectMask,
                   &ev);
        switch (ev.type) {
        case ConfigureRequest:
        case Expose:
        case MapRequest:
            handler[ev.type](&ev);
            break;
        case MotionNotify:
            if ((ev.xmotion.time - lasttime) <=
                (1000 / (doubledraw ? 240 : 120)))
                continue;
            lasttime = ev.xmotion.time;

            nw = MAX(ev.xmotion.x - ocx - 2 * c->bw + 1, 1);
            nh = MAX(ev.xmotion.y - ocy - 2 * c->bw + 1, 1);
            nx = c->x;
            ny = c->y;
            nw = MAX(ev.xmotion.x - ocx - 2 * c->bw + 1, 1);
            nh = MAX(ev.xmotion.y - ocy - 2 * c->bw + 1, 1);

            if (c->mon->wx + nw >= selmon->wx &&
                c->mon->wx + nw <= selmon->wx + selmon->ww &&
                c->mon->wy + nh >= selmon->wy &&
                c->mon->wy + nh <= selmon->wy + selmon->wh) {
                if (!c->isfloating && selmon->lt[selmon->sellt]->arrange &&
                    (abs(nw - c->w) > snap || abs(nh - c->h) > snap))
                    togglefloating(NULL);
            }

            if (!selmon->lt[selmon->sellt]->arrange || c->isfloating) {
                if (ev.xmotion.x < ocx + c->w) {
                    resize(c, nx, ny, nw, nw * (float)ocy2 / ocx2, 1);
                } else if (ev.xmotion.y < ocy + c->h) {
                    resize(c, nx, ny, nh * (float)ocx2 / ocy2, nh, 1);
                } else if (ev.xmotion.x > ocx + c->w + c->bw - 1 + 40) {
                    resize(c, nx, ny, nh * (float)ocx2 / ocy2, nh, 1);
                } else if (ev.xmotion.y > ocy + c->h + c->bw - 1 + 40) {
                    resize(c, nx, ny, nw, nw * (float)ocy2 / ocx2, 1);
                }
            }
            break;
        }
    } while (ev.type != ButtonRelease);
    XUngrabPointer(dpy, CurrentTime);
    while (XCheckMaskEvent(dpy, EnterWindowMask, &ev))
        ;

    if ((m = recttomon(c->x, c->y, c->w, c->h)) != selmon) {
        sendmon(c, m);
        unfocus(selmon->sel, 0);
        selmon = m;
        focus(NULL);
    }
}

void resizerequest(XEvent *e) {
    XResizeRequestEvent *ev = &e->xresizerequest;
    Client *i;

    if ((i = wintosystrayicon(ev->window))) {
        updatesystrayicongeom(i, ev->width, ev->height);
        resizebarwin(selmon);
        updatesystray();
    }
}

void restack(Monitor *m) {
    if (&overviewlayout == m->lt[m->sellt]->arrange)
        return;
    Client *c;
    XEvent ev;
    XWindowChanges wc;

    drawbar(m);
    if (!m->sel)
        return;
    if (m->sel->isfloating || !m->lt[m->sellt]->arrange)
        XRaiseWindow(dpy, m->sel->win);
    if (m->lt[m->sellt]->arrange) {
        wc.stack_mode = Below;
        wc.sibling = m->barwin;
        for (c = m->stack; c; c = c->snext)
            if (!c->isfloating && ISVISIBLE(c)) {
                XConfigureWindow(dpy, c->win, CWSibling | CWStackMode, &wc);
                wc.sibling = c->win;
            }
    }
    XSync(dpy, False);
    while (XCheckMaskEvent(dpy, EnterWindowMask, &ev))
        ;
}

void run(void) {
    XEvent ev;
    /* main event loop */
    XSync(dpy, False);
    while (running && !XNextEvent(dpy, &ev))
        if (handler[ev.type])
            handler[ev.type](&ev); /* call handler */
}

void runAutostart(void) {
    system("command -v instantautostart || { sleep 4 && notify-send "
           "'instantutils missing, please install instantutils!!!'; } &");
    system("instantautostart &");
}

void scan(void) {
    unsigned int num;
    Window d1, d2, *wins = NULL;
    XWindowAttributes wa;

    if (XQueryTree(dpy, root, &d1, &d2, &wins, &num)) {
        unsigned int i;
        for (i = 0; i < num; i++) {
            if (!XGetWindowAttributes(dpy, wins[i], &wa) ||
                wa.override_redirect || XGetTransientForHint(dpy, wins[i], &d1))
                continue;
            if (wa.map_state == IsViewable || getstate(wins[i]) == IconicState)
                manage(wins[i], &wa);
        }
        for (i = 0; i < num; i++) { /* now the transients */
            if (!XGetWindowAttributes(dpy, wins[i], &wa))
                continue;
            if (XGetTransientForHint(dpy, wins[i], &d1) &&
                (wa.map_state == IsViewable ||
                 getstate(wins[i]) == IconicState))
                manage(wins[i], &wa);
        }
        if (wins)
            XFree(wins);
    }
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
    } while (++i < LENGTH(tags));
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
    } while (ix >= x + selmon->mx && ++i < LENGTH(tags));
    return i;
}

// send client to another monitor
void sendmon(Client *c, Monitor *m) {
    int isscratchpad = 0;
    Monitor *prevmon = selmon;
    if (c->mon == m)
        return;

    prevmon = selmon;

    unfocus(c, 1);
    detach(c);
    detachstack(c);
    c->mon = m;
    // make scratchpad windows reappear on the other monitor scratchpad
    if (c->tags != (1 << 20)) {
        c->tags = m->tagset[m->seltags]; /* assign tags of target monitor */
        resetsticky(c);
    } else {
        isscratchpad = 1;
    }
    attach(c);
    attachstack(c);
    setclienttagprop(c);
    focus(NULL);
    if (!c->isfloating)
        arrange(NULL);
    if (isscratchpad && !c->mon->scratchvisible) {
        unfocus(selmon->sel, 0);
        selmon = m;
        togglescratchpad(NULL);
        focus(NULL);
        unfocus(selmon->sel, 0);
        selmon = prevmon;
        focus(NULL);
    }
}

void setclientstate(Client *c, long state) {
    long data[] = {state, None};

    XChangeProperty(dpy, c->win, wmatom[WMState], wmatom[WMState], 32,
                    PropModeReplace, (unsigned char *)data, 2);
}

int sendevent(Window w, Atom proto, int mask, long d0, long d1, long d2,
              long d3, long d4) {
    int n;
    Atom *protocols, mt;
    int exists = 0;
    XEvent ev;

    if (proto == wmatom[WMTakeFocus] || proto == wmatom[WMDelete]) {
        mt = wmatom[WMProtocols];
        if (XGetWMProtocols(dpy, w, &protocols, &n)) {
            while (!exists && n--)
                exists = protocols[n] == proto;
            XFree(protocols);
        }
    } else {
        exists = True;
        mt = proto;
    }
    if (exists) {
        ev.type = ClientMessage;
        ev.xclient.window = w;
        ev.xclient.message_type = mt;
        ev.xclient.format = 32;
        ev.xclient.data.l[0] = d0;
        ev.xclient.data.l[1] = d1;
        ev.xclient.data.l[2] = d2;
        ev.xclient.data.l[3] = d3;
        ev.xclient.data.l[4] = d4;
        XSendEvent(dpy, w, False, mask, &ev);
    }
    return exists;
}

void setfocus(Client *c) {
    if (!c->neverfocus) {
        XSetInputFocus(dpy, c->win, RevertToPointerRoot, CurrentTime);
        XChangeProperty(dpy, root, netatom[NetActiveWindow], XA_WINDOW, 32,
                        PropModeReplace, (unsigned char *)&(c->win), 1);
    }
    sendevent(c->win, wmatom[WMTakeFocus], NoEventMask, wmatom[WMTakeFocus],
              CurrentTime, 0, 0, 0);
}

void setfullscreen(Client *c, int fullscreen) {
    if (fullscreen && !c->isfullscreen) {
        XChangeProperty(dpy, c->win, netatom[NetWMState], XA_ATOM, 32,
                        PropModeReplace,
                        (unsigned char *)&netatom[NetWMFullscreen], 1);
        c->isfullscreen = 1;

        c->oldstate = c->isfloating;
        savebw(c);
        if (!c->isfakefullscreen) {
            c->bw = 0;
            if (!c->isfloating)
                animateclient(c, c->mon->mx, c->mon->my, c->mon->mw, c->mon->mh,
                              10, 0);
            resizeclient(c, c->mon->mx, c->mon->my, c->mon->mw, c->mon->mh);
            XRaiseWindow(dpy, c->win);
        }
        c->isfloating = 1;

    } else if (!fullscreen && c->isfullscreen) {
        XChangeProperty(dpy, c->win, netatom[NetWMState], XA_ATOM, 32,
                        PropModeReplace, (unsigned char *)0, 0);
        c->isfullscreen = 0;

        c->isfloating = c->oldstate;
        restorebw(c);
        c->x = c->oldx;
        c->y = c->oldy;
        c->w = c->oldw;
        c->h = c->oldh;

        if (!c->isfakefullscreen) {
            resizeclient(c, c->x, c->y, c->w, c->h);
            arrange(c->mon);
        }
    }
}

void commandprefix(const Arg *arg) {
    tagprefix = arg->ui;
    drawbar(selmon);
}

void commandlayout(const Arg *arg) {
    int layoutnumber;
    Arg *larg;
    if (arg->ui || arg->ui >= LENGTH(layouts))
        layoutnumber = arg->ui;
    else
        layoutnumber = 0;

    larg = &((Arg){.v = &layouts[layoutnumber]});
    setlayout(larg);
}

void setlayout(const Arg *arg) {
    int multimon;
    multimon = 0;
    if (tagprefix) {
        int i;
        Monitor *m;
        multimon = 1;
        for (m = mons; m; m = m->next) {
            for (i = 0; i < 20; ++i) {
                if (!arg || !arg->v || arg->v != m->lt[m->sellt])
                    m->pertag->sellts[i] ^= 1;
                if (arg && arg->v)
                    m->pertag->ltidxs[i][m->pertag->sellts[i]] =
                        (Layout *)arg->v;
            }
        }
        tagprefix = 0;
        setlayout(arg);
    } else {
        if (!arg || !arg->v || arg->v != selmon->lt[selmon->sellt])
            selmon->sellt = selmon->pertag->sellts[selmon->pertag->curtag] ^= 1;
        if (arg && arg->v)
            selmon->lt[selmon->sellt] =
                selmon->pertag->ltidxs[selmon->pertag->curtag][selmon->sellt] =
                    (Layout *)arg->v;
    }
    strncpy(selmon->ltsymbol, selmon->lt[selmon->sellt]->symbol,
            sizeof selmon->ltsymbol);
    if (selmon->sel)
        arrange(selmon);
    else
        drawbar(selmon);
    if (multimon) {
        Monitor *tmpmon;
        Monitor *m;
        tmpmon = selmon;
        multimon = 0;
        for (m = mons; m; m = m->next) {
            if (m != selmon) {
                selmon = m;
                setlayout(arg);
            }
        }
        selmon = tmpmon;
        focus(NULL);
    }
}

/* arg > 1.0 will set mfact absolutely */
void setmfact(const Arg *arg) {
    float f;
    int tmpanim = 0;
    if (!arg || !selmon->lt[selmon->sellt]->arrange)
        return;
    f = arg->f < 1.0 ? arg->f + selmon->mfact : arg->f - 1.0;
    if (f < 0.05 || f > 0.95)
        return;
    selmon->mfact = selmon->pertag->mfacts[selmon->pertag->curtag] = f;

    if (animated && clientcount() > 2) {
        tmpanim = 1;
        animated = 0;
    }

    arrange(selmon);
    if (tmpanim)
        animated = 1;
}

void load_xresources(void) {
    Display *display;
    char *resm;
    XrmDatabase db;
    ResourcePref *p;

    int i, u, q;

    display = XOpenDisplay(NULL);
    resm = XResourceManagerString(display);
    if (!resm)
        return;

    db = XrmGetStringDatabase(resm);


    for (i = 0; i < LENGTH(schemehovertypes); i++) {
        for (q = 0; q < LENGTH(schemecolortypes); q++) {
            for (u = 0; u < LENGTH(schemewindowtypes); u++) {
                char propname[100] = "";
                snprintf(propname, sizeof(propname), "%s.%s.win.%s",
                         schemehovertypes[i].name, schemewindowtypes[u].name,
                         schemecolortypes[q].name);

                // duplicate default value to avoid reading xresource into
                // multiple colors
                char *tmpstring = (char *)malloc((7 + 1) * sizeof(char));
                strcpy(tmpstring, windowcolors[schemehovertypes[i].type]
                                              [schemewindowtypes[u].type]
                                              [schemecolortypes[q].type]);

                windowcolors[schemehovertypes[i].type]
                            [schemewindowtypes[u].type]
                            [schemecolortypes[q].type] = tmpstring;

                resource_load(db, propname, STRING,
                              (void *)(windowcolors[schemehovertypes[i].type]
                                                   [schemewindowtypes[u].type]
                                                   [schemecolortypes[q].type]));
            }

            for (u = 0; u < LENGTH(schemetagtypes); u++) {
                char propname[100] = "";
                snprintf(propname, sizeof(propname), "%s.%s.tag.%s",
                         schemehovertypes[i].name, schemetagtypes[u].name,
                         schemecolortypes[q].name);

                char *tmpstring = (char *)malloc((7 + 1) * sizeof(char));
                
                strcpy(tmpstring, tagcolors[schemehovertypes[i].type]
                                              [schemetagtypes[u].type]
                                              [schemecolortypes[q].type]);

                tagcolors[schemehovertypes[i].type]
                            [schemetagtypes[u].type]
                            [schemecolortypes[q].type] = tmpstring;
                resource_load(db, propname, STRING,
                              (void *)(tagcolors[schemehovertypes[i].type]
                                                [schemetagtypes[u].type]
                                                [schemecolortypes[q].type]));
            }

            for (u = 0; u < LENGTH(schemeclosetypes); u++) {
                char propname[100] = "";
                snprintf(propname, sizeof(propname), "%s.%s.close.%s",
                         schemehovertypes[i].name, schemeclosetypes[u].name,
                         schemecolortypes[q].name);

                char *tmpstring = (char *)malloc((7 + 1) * sizeof(char));
                strcpy(tmpstring, closebuttoncolors[schemehovertypes[i].type]
                                              [schemeclosetypes[u].type]
                                              [schemecolortypes[q].type]);

                closebuttoncolors[schemehovertypes[i].type]
                            [schemeclosetypes[u].type]
                            [schemecolortypes[q].type] = tmpstring;
                resource_load(
                    db, propname, STRING,
                    (void *)(closebuttoncolors[schemehovertypes[i].type]
                                              [schemeclosetypes[u].type]
                                              [schemecolortypes[q].type]));
            }
        }
    }

    resource_load(db, "normal.border", STRING,
                  (void *)bordercolors[SchemeBorderNormal]);
    resource_load(db, "focus.tile.border", STRING,
                  (void *)bordercolors[SchemeBorderTileFocus]);
    resource_load(db, "focus.float.border", STRING,
                  (void *)bordercolors[SchemeBorderFloatFocus]);
    resource_load(db, "snap.border", STRING,
                  (void *)bordercolors[SchemeBorderSnap]);

    resource_load(db, "status.fg", STRING, (void *)statusbarcolors[ColFg]);
    resource_load(db, "status.bg", STRING, (void *)statusbarcolors[ColBg]);
    resource_load(db, "status.detail", STRING,
                  (void *)statusbarcolors[ColDetail]);

    for (p = resources; p < resources + LENGTH(resources); p++)
        resource_load(db, p->name, p->type, p->dst);

    XCloseDisplay(display);
}

void setup(void) {
    int i;
    int u;

    XSetWindowAttributes wa;
    Atom utf8string;

    /* clean up any zombies immediately */
    sigchld(0);

    /* init screen */
    screen = DefaultScreen(dpy);
    sw = DisplayWidth(dpy, screen);
    sh = DisplayHeight(dpy, screen);
    root = RootWindow(dpy, screen);

    if (strlen(xresourcesfont) > 3) {
        fonts[0] = xresourcesfont;
        fprintf(stderr, "manual font %s", xresourcesfont);
    }

    drw = drw_create(dpy, screen, root, sw, sh);
    if (!drw_fontset_create(drw, fonts, LENGTH(fonts)))
        die("no fonts could be loaded.");
    lrpad = drw->fonts->h;
    if (barheight)
        bh = drw->fonts->h + barheight;
    else
        bh = drw->fonts->h + 12;
    updategeom();
    /* init atoms */
    utf8string = XInternAtom(dpy, "UTF8_STRING", False);
    wmatom[WMProtocols] = XInternAtom(dpy, "WM_PROTOCOLS", False);
    wmatom[WMDelete] = XInternAtom(dpy, "WM_DELETE_WINDOW", False);
    wmatom[WMState] = XInternAtom(dpy, "WM_STATE", False);
    wmatom[WMTakeFocus] = XInternAtom(dpy, "WM_TAKE_FOCUS", False);
    netatom[NetActiveWindow] = XInternAtom(dpy, "_NET_ACTIVE_WINDOW", False);
    netatom[NetSupported] = XInternAtom(dpy, "_NET_SUPPORTED", False);
    netatom[NetSystemTray] = XInternAtom(dpy, "_NET_SYSTEM_TRAY_S0", False);
    netatom[NetSystemTrayOP] =
        XInternAtom(dpy, "_NET_SYSTEM_TRAY_OPCODE", False);
    netatom[NetSystemTrayOrientation] =
        XInternAtom(dpy, "_NET_SYSTEM_TRAY_ORIENTATION", False);
    netatom[NetSystemTrayOrientationHorz] =
        XInternAtom(dpy, "_NET_SYSTEM_TRAY_ORIENTATION_HORZ", False);
    netatom[NetWMName] = XInternAtom(dpy, "_NET_WM_NAME", False);
    netatom[NetWMState] = XInternAtom(dpy, "_NET_WM_STATE", False);
    netatom[NetWMCheck] = XInternAtom(dpy, "_NET_SUPPORTING_WM_CHECK", False);
    netatom[NetWMFullscreen] =
        XInternAtom(dpy, "_NET_WM_STATE_FULLSCREEN", False);
    netatom[NetWMWindowType] = XInternAtom(dpy, "_NET_WM_WINDOW_TYPE", False);
    netatom[NetWMWindowTypeDialog] =
        XInternAtom(dpy, "_NET_WM_WINDOW_TYPE_DIALOG", False);
    netatom[NetClientList] = XInternAtom(dpy, "_NET_CLIENT_LIST", False);
    netatom[NetClientInfo] = XInternAtom(dpy, "_NET_CLIENT_INFO", False);
    motifatom = XInternAtom(dpy, "_MOTIF_WM_HINTS", False);

    xatom[Manager] = XInternAtom(dpy, "MANAGER", False);
    xatom[Xembed] = XInternAtom(dpy, "_XEMBED", False);
    xatom[XembedInfo] = XInternAtom(dpy, "_XEMBED_INFO", False);
    /* init cursors */
    cursor[CurNormal] = drw_cur_create(drw, XC_left_ptr);
    cursor[CurResize] = drw_cur_create(drw, XC_crosshair);
    cursor[CurMove] = drw_cur_create(drw, XC_fleur);
    cursor[CurClick] = drw_cur_create(drw, XC_hand1);
    cursor[CurVert] = drw_cur_create(drw, XC_sb_v_double_arrow);
    cursor[CurHor] = drw_cur_create(drw, XC_sb_h_double_arrow);
    cursor[CurBL] = drw_cur_create(drw, XC_bottom_left_corner);
    cursor[CurBR] = drw_cur_create(drw, XC_bottom_right_corner);
    cursor[CurTL] = drw_cur_create(drw, XC_top_left_corner);
    cursor[CurTR] = drw_cur_create(drw, XC_top_right_corner);

    /* scheme = ecalloc(LENGTH(colors) + 1, sizeof(Clr *)); */
    /* scheme[LENGTH(colors)] = drw_scm_create(drw, colors[0], 4); */

    /* for (i = 0; i < LENGTH(colors); i++) */
    /*     scheme[i] = drw_scm_create(drw, colors[i], 4); */

    borderscheme = drw_scm_create(drw, bordercolors, 4);
    statusscheme = drw_scm_create(drw, statusbarcolors, 3);

    tagscheme = ecalloc(2, sizeof(Clr **));
    for (i = 0; i < LENGTH(tagcolors); i++) {
        tagscheme[i] = ecalloc(LENGTH(tagcolors[i]) + 1, sizeof(Clr **));
        for (u = 0; u < LENGTH(tagcolors[i]); u++) {
            tagscheme[i][u] = drw_scm_create(drw, tagcolors[i][u], 3);
        }
    }

    windowscheme = ecalloc(2, sizeof(Clr **));
    for (i = 0; i < LENGTH(windowcolors); i++) {
        windowscheme[i] = ecalloc(LENGTH(windowcolors[i]) + 1, sizeof(Clr **));
        for (u = 0; u < LENGTH(windowcolors[i]); u++) {
            windowscheme[i][u] = drw_scm_create(drw, windowcolors[i][u], 3);
        }
    }

    closebuttonscheme = ecalloc(2, sizeof(Clr **));
    for (i = 0; i < LENGTH(closebuttoncolors); i++) {
        closebuttonscheme[i] =
            ecalloc(LENGTH(closebuttoncolors[i]) + 1, sizeof(Clr **));
        for (u = 0; u < LENGTH(closebuttoncolors[i]); u++) {
            closebuttonscheme[i][u] =
                drw_scm_create(drw, closebuttoncolors[i][u], 3);
        }
    }

    /* init system tray */
    updatesystray();
    /* init bars */
    verifytagsxres();
    updatebars();
    updatestatus();
    /* supporting window for NetWMCheck */
    wmcheckwin = XCreateSimpleWindow(dpy, root, 0, 0, 1, 1, 0, 0, 0);
    XChangeProperty(dpy, wmcheckwin, netatom[NetWMCheck], XA_WINDOW, 32,
                    PropModeReplace, (unsigned char *)&wmcheckwin, 1);
    XChangeProperty(dpy, wmcheckwin, netatom[NetWMName], utf8string, 8,
                    PropModeReplace, (unsigned char *)"dwm", 3);
    XChangeProperty(dpy, root, netatom[NetWMCheck], XA_WINDOW, 32,
                    PropModeReplace, (unsigned char *)&wmcheckwin, 1);
    /* EWMH support per view */
    XChangeProperty(dpy, root, netatom[NetSupported], XA_ATOM, 32,
                    PropModeReplace, (unsigned char *)netatom, NetLast);
    XDeleteProperty(dpy, root, netatom[NetClientList]);
    XDeleteProperty(dpy, root, netatom[NetClientInfo]);
    /* select events */
    wa.cursor = cursor[CurNormal]->cursor;
    wa.event_mask = SubstructureRedirectMask | SubstructureNotifyMask |
                    ButtonPressMask | PointerMotionMask | EnterWindowMask |
                    LeaveWindowMask | StructureNotifyMask | PropertyChangeMask;
    XChangeWindowAttributes(dpy, root, CWEventMask | CWCursor, &wa);
    XSelectInput(dpy, root, wa.event_mask);
    grabkeys();
    focus(NULL);
}

void seturgent(Client *c, int urg) {
    XWMHints *wmh;

    c->isurgent = urg;
    if (!(wmh = XGetWMHints(dpy, c->win)))
        return;
    wmh->flags =
        urg ? (wmh->flags | XUrgencyHint) : (wmh->flags & ~XUrgencyHint);
    XSetWMHints(dpy, c->win, wmh);
    XFree(wmh);
}

// unminimize window
void show(Client *c) {
    int x, y, w, h;
    if (!c || !HIDDEN(c))
        return;

    x = c->x;
    y = c->y;
    w = c->w;
    h = c->h;

    XMapWindow(dpy, c->win);
    setclientstate(c, NormalState);
    resize(c, x, -50, w, h, 0);
    XRaiseWindow(dpy, c->win);
    animateclient(c, x, y, 0, 0, 14, 0);
    arrange(c->mon);
}

void showhide(Client *c) {
    if (!c)
        return;
    if (ISVISIBLE(c)) {
        /* show clients top down */
        XMoveWindow(dpy, c->win, c->x, c->y);
        if ((!c->mon->lt[c->mon->sellt]->arrange || c->isfloating) &&
            (!c->isfullscreen || c->isfakefullscreen))
            resize(c, c->x, c->y, c->w, c->h, 0);
        showhide(c->snext);
    } else {
        /* hide clients bottom up */
        showhide(c->snext);
        XMoveWindow(dpy, c->win, WIDTH(c) * -2, c->y);
    }
}

void sigchld(int unused) {
    if (signal(SIGCHLD, sigchld) == SIG_ERR)
        die("can't install SIGCHLD handler:");
    while (0 < waitpid(-1, NULL, WNOHANG))
        ;
}

void spawn(const Arg *arg) {
    if (arg->v == instantmenucmd)
        instantmenumon[0] = '0' + selmon->num;
    if (fork() == 0) {
        if (dpy)
            close(ConnectionNumber(dpy));
        setsid();
        execvp(((char **)arg->v)[0], (char **)arg->v);
        fprintf(stderr, "instantwm: execvp %s", ((char **)arg->v)[0]);
        perror(" failed");
        exit(EXIT_SUCCESS);
    }
}

int computeprefix(const Arg *arg) {
    if (tagprefix && arg->ui) {
        tagprefix = 0;
        return arg->ui << 10;
    } else {
        return arg->ui;
    }
}


void
setclienttagprop(Client *c)
{
	long data[] = { (long) c->tags, (long) c->mon->num };
	XChangeProperty(dpy, c->win, netatom[NetClientInfo], XA_CARDINAL, 32,
			PropModeReplace, (unsigned char *) data, 2);
}

void tag(const Arg *arg) {
    int ui = computeprefix(arg);
    Client *c;
    if (selmon->sel && ui & TAGMASK) {
        if (selmon->sel->tags == 1 << 20)
            selmon->sel->issticky = 0;
        c = selmon->sel;
        selmon->sel->tags = ui & TAGMASK;
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
    if (selmon->sel && ui & TAGMASK) {
        for (c = selmon->clients; c; c = c->next) {
            if (!(c->tags & 1 << (selmon->pertag->curtag - 1)))
                continue;
            if (c->tags == 1 << 20)
                c->issticky = 0;
            c->tags = ui & TAGMASK;
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
    unsigned int newtag = ui & TAGMASK;
    unsigned int curtag = selmon->tagset[selmon->seltags];

    if (newtag == curtag || !curtag || (curtag & (curtag - 1)))
        return;

    for (Client *c = selmon->clients; c != NULL; c = c->next) {
        if (selmon->overlay == c)
		{
			if (ISVISIBLE(c))
				hideoverlay();
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

void setoverlaymode(int mode) {
    Monitor *m;
    for (m = mons; m; m = m->next) {
        m->overlaymode = mode;
    }

    if (!selmon->overlay)
        return;

    if (mode == 0 || mode == 2)
        selmon->overlay->h = selmon->wh / 3;
    else
        selmon->overlay->w = selmon->ww / 3;

    if (selmon->overlaystatus) {
        hideoverlay();
        showoverlay();
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
        __builtin_popcount(selmon->tagset[selmon->seltags] & TAGMASK) == 1 &&
        selmon->tagset[selmon->seltags] > 1) {
        selmon->sel->tags >>= offset;
        focus(NULL);
        arrange(selmon);
    }
    c->x = oldx;
}

void uppress(const Arg *arg) {
    if (!selmon->sel)
        return;
    if (selmon->sel == selmon->overlay) {
        setoverlaymode(0);
        return;
    }
    if (selmon->sel->isfloating) {
        togglefloating(NULL);
        return;
    } else {
        hide(selmon->sel);
        return;
    }
}

void downpress(const Arg *arg) {
    if (unhideone())
        return;
    if (!selmon->sel)
        return;

    if (selmon->sel->snapstatus) {
        resetsnap(selmon->sel);
        return;
    }

    if (selmon->sel == selmon->overlay) {
        setoverlaymode(2);
        return;
    }
    if (!selmon->sel->isfloating) {
        togglefloating(NULL);
        return;
    }
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
        __builtin_popcount(selmon->tagset[selmon->seltags] & TAGMASK) == 1 &&
        selmon->tagset[selmon->seltags] & (TAGMASK >> 1)) {
        selmon->sel->tags <<= offset;
        focus(NULL);
        arrange(selmon);
    }
    c->x = oldx;
}

void ctrltoggle(int *value, int arg) {
    if (arg == 0 || arg == 2) {
        *value = !*value;
    } else {
        if (arg == 1)
            *value = 0;
        else
            *value = 1;
    }
}

void setspecialnext(const Arg *arg) { specialnext = arg->ui; }

// toggle tag icon view
void togglealttag(const Arg *arg) {
    ctrltoggle(&showalttag, arg->ui);

    Monitor *m;
    for (m = mons; m; m = m->next)
        drawbar(m);

    tagwidth = gettagwidth();
}

void alttabfree(const Arg *arg) {
    ctrltoggle(&freealttab, arg->ui);
    grabkeys();
}

// make client show on all tags
void togglesticky(const Arg *arg) {
    if (!selmon->sel)
        return;
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
    if (!selmon->sel)
        return;
    c = selmon->sel;
    width = c->bw;
    c->bw = arg->i;
    d = width - c->bw;
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

void togglefakefullscreen(const Arg *arg) {
    if (selmon->sel->isfullscreen) {
        if (selmon->sel->isfakefullscreen) {
            resizeclient(selmon->sel, selmon->mx + borderpx,
                         selmon->my + borderpx, selmon->mw - 2 * borderpx,
                         selmon->mh - 2 * borderpx);
            XRaiseWindow(dpy, selmon->sel->win);
        } else {
            selmon->sel->bw = selmon->sel->oldbw;
        }
    }

    selmon->sel->isfakefullscreen = !selmon->sel->isfakefullscreen;
}

// lock prevents windows from getting closed until unlocked
void togglelocked(const Arg *arg) {
    if (!selmon->sel)
        return;
    selmon->sel->islocked = !selmon->sel->islocked;
    drawbar(selmon);
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

void centerwindow() {
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

// toggle vacant tags
void toggleshowtags(const Arg *arg) {
    int showtags = selmon->showtags;
    ctrltoggle(&showtags, arg->ui);
    selmon->showtags = showtags;
    tagwidth = gettagwidth();
    drawbar(selmon);
}

void togglebar(const Arg *arg) {
    int tmpnoanim;
    if (animated && clientcount() > 6) {
        animated = 0;
        tmpnoanim = 1;
    } else {
        tmpnoanim = 0;
    }

    selmon->showbar = selmon->pertag->showbars[selmon->pertag->curtag] =
        !selmon->showbar;
    updatebarpos(selmon);
    resizebarwin(selmon);
    if (showsystray) {
        XWindowChanges wc;
        if (!selmon->showbar)
            wc.y = -bh;
        else {
            wc.y = 0;
            if (!selmon->topbar)
                wc.y = selmon->mh - bh;
        }
        XConfigureWindow(dpy, systray->win, CWY, &wc);
    }
    arrange(selmon);
    if (tmpnoanim)
        animated = 1;
    if (selmon->overlaystatus) {
        tmpnoanim = animated;
        animated = 0;
        selmon->overlaystatus = 0;
        showoverlay();
        animated = tmpnoanim;
    }
}

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

void toggletag(const Arg *arg) {
    int ui = computeprefix(arg);
    unsigned int newtags;

    if (!selmon->sel)
        return;

    if (selmon->sel->tags == 1 << 20) {
        tag(arg);
        return;
    }

    newtags = selmon->sel->tags ^ (ui & TAGMASK);
    if (newtags) {
        selmon->sel->tags = newtags;
        setclienttagprop(selmon->sel);
        focus(NULL);
        arrange(selmon);
    }
}

void togglescratchpad(const Arg *arg) {
    Client *c;
    Client *activescratchpad;
    activescratchpad = NULL;
    int scratchexists;
    scratchexists = 0;
    if (&overviewlayout == selmon->lt[selmon->sellt]->arrange) {
        return;
    }

    if (selmon->scratchvisible)
        selmon->scratchvisible = 0;
    else
        selmon->scratchvisible = 1;

    for (c = selmon->clients; c; c = c->next) {
        if (c->tags & 1 << 20) {
            c->tags = 1 << 20;
            if (!scratchexists)
                scratchexists = 1;
            c->issticky = selmon->scratchvisible;
            if (c == selmon->fullscreen)
                tempfullscreen();
            if (!c->isfloating)
                c->isfloating = 1;
        }
    }

    if (!scratchexists) {
        // spawn scratchpad
        spawn(&((Arg){.v = termscratchcmd}));
        return;
    }

    arrange(selmon);
    if (selmon->scratchvisible) {

        for (c = selmon->clients; c; c = c->next) {
            if (c->tags & 1 << 20) {
                XRaiseWindow(dpy, c->win);
            }
        }

        if (selmon->activescratchpad) {
            activescratchpad = selmon->activescratchpad;
        } else {
            for (c = selmon->clients; c; c = c->next) {
                if (c->tags == 1 << 20) {
                    if ((!selmon->sel || !selmon->sel->isfullscreen) &&
                        c->issticky) {
                        activescratchpad = c;
                    } else {
                        arrange(selmon);
                    }
                    break;
                }
            }
        }
        if (activescratchpad) {
            selmon->sel = activescratchpad;
            arrange(selmon);
            focus(activescratchpad);
            // if focusfollowsmouse is off, the mouse doesn't
            // need to move to keep focus on the scratchpad
            if (focusfollowsmouse) {
                warp(activescratchpad);
            }
        }
    } else {
        focus(NULL);
        arrange(selmon);
    }
}

void createscratchpad(const Arg *arg) {
    Client *c;
    if (!selmon->sel)
        return;
    c = selmon->sel;

    // turn scratchpad back into normal window
    if (c->tags == 1 << 20) {
        tag(&((Arg){.ui = 1 << (selmon->pertag->curtag - 1)}));
        return;
    }

    c->tags = 1 << 20;
    c->issticky = selmon->scratchvisible;
    if (!c->isfloating)
        togglefloating(NULL);
    else
        arrange(selmon);
    focus(NULL);
    if (!selmon->scratchvisible) {
        togglescratchpad(NULL);
    }
}

void toggleview(const Arg *arg) {
    unsigned int newtagset =
        selmon->tagset[selmon->seltags] ^ (arg->ui & TAGMASK);
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

// minimize window
void hidewin(const Arg *arg) {
    if (!selmon->sel)
        return;
    Client *c = selmon->sel;
    if (HIDDEN(c))
        return;
    hide(c);
}

// fixes drawing issues with wine games
void redrawwin(const Arg *arg) {
    int tmpanimated = 0;
    if (!selmon->sel)
        return;
    Client *c = selmon->sel;
    if (HIDDEN(c))
        return;
    if (animated) {
        tmpanimated = 1;
        animated = 0;
    }

    hide(c);
    show(c);
    if (tmpanimated)
        animated = 1;
}

void unhideall(const Arg *arg) {

    Client *c;
    for (c = selmon->clients; c; c = c->next) {
        if (ISVISIBLE(c) && HIDDEN(c))
            show(c);
    }
    focus(c);
    restack(selmon);
}

void unfocus(Client *c, int setfocus) {
    if (!c)
        return;
    lastclient = c;
    grabbuttons(c, 0);
    XSetWindowBorder(dpy, c->win, borderscheme[SchemeBorderNormal].pixel);
    if (setfocus) {
        XSetInputFocus(dpy, root, RevertToPointerRoot, CurrentTime);
        XDeleteProperty(dpy, root, netatom[NetActiveWindow]);
    }
}

void unmanage(Client *c, int destroyed) {
    Monitor *m = c->mon;
    XWindowChanges wc;
    if (c == selmon->overlay) {
        Monitor *tm;
        for (tm = mons; tm; tm = tm->next) {
            tm->overlay = NULL;
        }
    }

    detach(c);
    detachstack(c);
    if (!destroyed) {
        wc.border_width = c->oldbw;
        XGrabServer(dpy); /* avoid race conditions */
        XSetErrorHandler(xerrordummy);
        XConfigureWindow(dpy, c->win, CWBorderWidth, &wc); /* restore border */
        XUngrabButton(dpy, AnyButton, AnyModifier, c->win);
        setclientstate(c, WithdrawnState);
        XSync(dpy, False);
        XSetErrorHandler(xerror);
        XUngrabServer(dpy);
    }
    free(c);
    focus(NULL);
    updateclientlist();
    arrange(m);
}

void unmapnotify(XEvent *e) {
    Client *c;
    XUnmapEvent *ev = &e->xunmap;

    if ((c = wintoclient(ev->window))) {
        if (ev->send_event)
            setclientstate(c, WithdrawnState);
        else
            unmanage(c, 0);
    } else if ((c = wintosystrayicon(ev->window))) {
        /* KLUDGE! sometimes icons occasionally unmap their windows, but do
         * _not_ destroy them. We map those windows back */
        XMapRaised(dpy, c->win);
        updatesystray();
    }
}

void verifytagsxres(void) {
    for (int i = 0; i < 9; i++) {
        int len = strlen(tags[i]);
        if (len > MAX_TAGLEN - 1 || len == 0) {
            strcpy((char *)&tags[i], "Xres err");
        }
    }
}

void updatebars(void) {
    unsigned int w;
    Monitor *m;
    XSetWindowAttributes wa = {.override_redirect = True,
                               .background_pixmap = ParentRelative,
                               .event_mask = ButtonPressMask | ExposureMask};
    XClassHint ch = {"dwm", "dwm"};
    for (m = mons; m; m = m->next) {
        if (m->barwin)
            continue;
        w = m->ww;
        if (showsystray && m == systraytomon(m))
            w -= getsystraywidth();
        m->barwin = XCreateWindow(
            dpy, root, m->wx, m->by, w, bh, 0, DefaultDepth(dpy, screen),
            CopyFromParent, DefaultVisual(dpy, screen),
            CWOverrideRedirect | CWBackPixmap | CWEventMask, &wa);
        // XDefineCursor(dpy, m->barwin, cursor[CurNormal]->cursor);
        if (showsystray && m == systraytomon(m))
            XMapRaised(dpy, systray->win);
        XMapRaised(dpy, m->barwin);
        XSetClassHint(dpy, m->barwin, &ch);
    }
}

void updatebarpos(Monitor *m) {
    m->wy = m->my;
    m->wh = m->mh;
    if (m->showbar) {
        m->wh -= bh;
        m->by = m->topbar ? m->wy : m->wy + m->wh;
        m->wy = m->topbar ? m->wy + bh : m->wy;
    } else
        m->by = -bh;
}

void updateclientlist() {
    Client *c;
    Monitor *m;

    XDeleteProperty(dpy, root, netatom[NetClientList]);
    for (m = mons; m; m = m->next)
        for (c = m->clients; c; c = c->next)
            XChangeProperty(dpy, root, netatom[NetClientList], XA_WINDOW, 32,
                            PropModeAppend, (unsigned char *)&(c->win), 1);
}

int updategeom(void) {
    int dirty = 0;

#ifdef XINERAMA
    if (XineramaIsActive(dpy)) {
        int i, j, n, nn;
        Client *c;
        Monitor *m;
        XineramaScreenInfo *info = XineramaQueryScreens(dpy, &nn);
        XineramaScreenInfo *unique = NULL;

        for (n = 0, m = mons; m; m = m->next, n++)
            ;
        /* only consider unique geometries as separate screens */
        unique = ecalloc(nn, sizeof(XineramaScreenInfo));
        for (i = 0, j = 0; i < nn; i++)
            if (isuniquegeom(unique, j, &info[i]))
                memcpy(&unique[j++], &info[i], sizeof(XineramaScreenInfo));
        XFree(info);
        nn = j;
        for (i = n; i < nn; i++) {
            for (m = mons; m && m->next; m = m->next)
                ;
            if (m)
                m->next = createmon();
            else
                mons = createmon();
        }
        for (i = 0, m = mons; i < nn && m; m = m->next, i++)
            if (i >= n || unique[i].x_org != m->mx ||
                unique[i].y_org != m->my || unique[i].width != m->mw ||
                unique[i].height != m->mh) {
                dirty = 1;
                m->num = i;
                m->mx = m->wx = unique[i].x_org;
                m->my = m->wy = unique[i].y_org;
                m->mw = m->ww = unique[i].width;
                m->mh = m->wh = unique[i].height;
                updatebarpos(m);
            }
        for (i = nn; i < n; i++) {
            for (m = mons; m && m->next; m = m->next)
                ;
            while ((c = m->clients)) {
                dirty = 1;
                m->clients = c->next;
                detachstack(c);
                c->mon = mons;
                attach(c);
                attachstack(c);
            }
            if (m == selmon)
                selmon = mons;
            cleanupmon(m);
        }
        free(unique);
    } else
#endif /* XINERAMA */
    {  /* default monitor setup */
        if (!mons)
            mons = createmon();
        if (mons->mw != sw || mons->mh != sh) {
            dirty = 1;
            mons->mw = mons->ww = sw;
            mons->mh = mons->wh = sh;
            updatebarpos(mons);
        }
    }
    if (dirty) {
        selmon = mons;
        selmon = wintomon(root);
    }
    return dirty;
}

// fix issues with custom window borders
void updatemotifhints(Client *c) {
    Atom real;
    int format;
    unsigned char *p = NULL;
    unsigned long n, extra;
    unsigned long *motif;
    int width, height;

    if (!decorhints)
        return;

    if (XGetWindowProperty(dpy, c->win, motifatom, 0L, 5L, False, motifatom,
                           &real, &format, &n, &extra, &p) == Success &&
        p != NULL) {
        motif = (unsigned long *)p;
        if (motif[MWM_HINTS_FLAGS_FIELD] & MWM_HINTS_DECORATIONS) {
            width = WIDTH(c);
            height = HEIGHT(c);

            if (motif[MWM_HINTS_DECORATIONS_FIELD] & MWM_DECOR_ALL ||
                motif[MWM_HINTS_DECORATIONS_FIELD] & MWM_DECOR_BORDER ||
                motif[MWM_HINTS_DECORATIONS_FIELD] & MWM_DECOR_TITLE)
                c->bw = c->oldbw = borderpx;
            else
                c->bw = c->oldbw = 0;

            resize(c, c->x, c->y, width - (2 * c->bw), height - (2 * c->bw), 0);
        }
        XFree(p);
    }
}

void updatenumlockmask(void) {
    unsigned int i, j;
    XModifierKeymap *modmap;

    numlockmask = 0;
    modmap = XGetModifierMapping(dpy);
    for (i = 0; i < 8; i++)
        for (j = 0; j < modmap->max_keypermod; j++)
            if (modmap->modifiermap[i * modmap->max_keypermod + j] ==
                XKeysymToKeycode(dpy, XK_Num_Lock))
                numlockmask = (1 << i);
    XFreeModifiermap(modmap);
}

void updatesizehints(Client *c) {
    long msize;
    XSizeHints size;

    if (!XGetWMNormalHints(dpy, c->win, &size, &msize))
        /* size is uninitialized, ensure that size.flags aren't used */
        size.flags = PSize;
    if (size.flags & PBaseSize) {
        c->basew = size.base_width;
        c->baseh = size.base_height;
    } else if (size.flags & PMinSize) {
        c->basew = size.min_width;
        c->baseh = size.min_height;
    } else
        c->basew = c->baseh = 0;
    if (size.flags & PResizeInc) {
        c->incw = size.width_inc;
        c->inch = size.height_inc;
    } else
        c->incw = c->inch = 0;
    if (size.flags & PMaxSize) {
        c->maxw = size.max_width;
        c->maxh = size.max_height;
    } else
        c->maxw = c->maxh = 0;
    if (size.flags & PMinSize) {
        c->minw = size.min_width;
        c->minh = size.min_height;
    } else if (size.flags & PBaseSize) {
        c->minw = size.base_width;
        c->minh = size.base_height;
    } else
        c->minw = c->minh = 0;
    if (size.flags & PAspect) {
        c->mina = (float)size.min_aspect.y / size.min_aspect.x;
        c->maxa = (float)size.max_aspect.x / size.max_aspect.y;
    } else
        c->maxa = c->mina = 0.0;
    c->isfixed =
        (c->maxw && c->maxh && c->maxw == c->minw && c->maxh == c->minh);
    c->hintsvalid = 1;
}

void updatestatus(void) {
    if (!gettextprop(root, XA_WM_NAME, stext, sizeof(stext)))
        strcpy(stext, "instantwm-" VERSION);
    drawbar(selmon);
    updatesystray();
}

void updatesystrayicongeom(Client *i, int w, int h) {
    if (i) {
        i->h = bh;
        if (w == h)
            i->w = bh;
        else if (h == bh)
            i->w = w;
        else
            i->w = (int)((float)bh * ((float)w / (float)h));
        applysizehints(i, &(i->x), &(i->y), &(i->w), &(i->h), False);
        /* force icons into the systray dimenons if they don't want to */
        if (i->h > bh) {
            if (i->w == i->h)
                i->w = bh;
            else
                i->w = (int)((float)bh * ((float)i->w / (float)i->h));
            i->h = bh;
        }
    }
}

void updatesystrayiconstate(Client *i, XPropertyEvent *ev) {
    long flags;
    int code = 0;

    if (!showsystray || !i || ev->atom != xatom[XembedInfo] ||
        !(flags = getatomprop(i, xatom[XembedInfo])))
        return;

    if (flags & XEMBED_MAPPED && !i->tags) {
        i->tags = 1;
        code = XEMBED_WINDOW_ACTIVATE;
        XMapRaised(dpy, i->win);
        setclientstate(i, NormalState);
    } else if (!(flags & XEMBED_MAPPED) && i->tags) {
        i->tags = 0;
        code = XEMBED_WINDOW_DEACTIVATE;
        XUnmapWindow(dpy, i->win);
        setclientstate(i, WithdrawnState);
    } else
        return;
    sendevent(i->win, xatom[Xembed], StructureNotifyMask, CurrentTime, code, 0,
              systray->win, XEMBED_EMBEDDED_VERSION);
}

void updatesystray(void) {
    XSetWindowAttributes wa;
    XWindowChanges wc;
    Client *i;
    Monitor *m = systraytomon(NULL);
    unsigned int x = m->mx + m->mw;
    unsigned int w = 1;

    if (!showsystray)
        return;
    if (!systray) {
        /* init systray */
        if (!(systray = (Systray *)calloc(1, sizeof(Systray))))
            die("fatal: could not malloc() %u bytes\n", sizeof(Systray));
        systray->win = XCreateSimpleWindow(
            dpy, root, x, m->by, w, bh, 0, 0,
            tagscheme[SchemeNoHover][SchemeTagFilled][ColBg].pixel);
        wa.event_mask = ButtonPressMask | ExposureMask;
        wa.override_redirect = True;
        wa.background_pixel = statusscheme[ColBg].pixel;
        XSelectInput(dpy, systray->win, SubstructureNotifyMask);
        XChangeProperty(dpy, systray->win, netatom[NetSystemTrayOrientation],
                        XA_CARDINAL, 32, PropModeReplace,
                        (unsigned char *)&netatom[NetSystemTrayOrientationHorz],
                        1);
        XChangeWindowAttributes(dpy, systray->win,
                                CWEventMask | CWOverrideRedirect | CWBackPixel,
                                &wa);
        XMapRaised(dpy, systray->win);
        XSetSelectionOwner(dpy, netatom[NetSystemTray], systray->win,
                           CurrentTime);
        if (XGetSelectionOwner(dpy, netatom[NetSystemTray]) == systray->win) {
            sendevent(root, xatom[Manager], StructureNotifyMask, CurrentTime,
                      netatom[NetSystemTray], systray->win, 0, 0);
            XSync(dpy, False);
        } else {
            fprintf(stderr, "instantwm: unable to obtain system tray.\n");
            free(systray);
            systray = NULL;
            return;
        }
    }
    for (w = 0, i = systray->icons; i; i = i->next) {
        /* make sure the background color stays the same */
        wa.background_pixel = statusscheme[ColBg].pixel;
        XChangeWindowAttributes(dpy, i->win, CWBackPixel, &wa);
        XMapRaised(dpy, i->win);
        w += systrayspacing;
        i->x = w;
        XMoveResizeWindow(dpy, i->win, i->x, 0, i->w, i->h);
        w += i->w;
        if (i->mon != m)
            i->mon = m;
    }
    w = w ? w + systrayspacing : 1;
    x -= w;
    XMoveResizeWindow(dpy, systray->win, x, m->by, w, bh);
    wc.x = x;
    wc.y = m->by;
    wc.width = w;
    wc.height = bh;
    wc.stack_mode = Above;
    wc.sibling = m->barwin;
    XConfigureWindow(dpy, systray->win,
                     CWX | CWY | CWWidth | CWHeight | CWSibling | CWStackMode,
                     &wc);
    XMapWindow(dpy, systray->win);
    XMapSubwindows(dpy, systray->win);
    /* redraw background */
    XSetForeground(dpy, drw->gc, statusscheme[ColBg].pixel);
    XFillRectangle(dpy, systray->win, drw->gc, 0, 0, w, bh);
    XSync(dpy, False);
}

void updatetitle(Client *c) {
    if (!gettextprop(c->win, netatom[NetWMName], c->name, sizeof c->name))
        gettextprop(c->win, XA_WM_NAME, c->name, sizeof c->name);
    if (c->name[0] == '\0') /* hack to mark broken clients */
        strcpy(c->name, broken);
}

void updatewindowtype(Client *c) {
    Atom state = getatomprop(c, netatom[NetWMState]);
    Atom wtype = getatomprop(c, netatom[NetWMWindowType]);

    if (state == netatom[NetWMFullscreen])
        setfullscreen(c, 1);
    if (wtype == netatom[NetWMWindowTypeDialog])
        c->isfloating = 1;
}

void updatewmhints(Client *c) {
    XWMHints *wmh;

    if ((wmh = XGetWMHints(dpy, c->win))) {
        if (c == selmon->sel && wmh->flags & XUrgencyHint) {
            wmh->flags &= ~XUrgencyHint;
            XSetWMHints(dpy, c->win, wmh);
        } else
            c->isurgent = (wmh->flags & XUrgencyHint) ? 1 : 0;
        if (wmh->flags & InputHint)
            c->neverfocus = !wmh->input;
        else
            c->neverfocus = 0;
        XFree(wmh);
    }
}

void view(const Arg *arg) {

    int ui = computeprefix(arg);
    int i;
    printf("%d\n", (int)(arg->ui));

    selmon->seltags ^= 1; /* toggle sel tagset */
    if (ui & TAGMASK) {
        selmon->tagset[selmon->seltags] = ui & TAGMASK;

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
    } else {
        unsigned int tmptag;
        tmptag = selmon->pertag->prevtag;
        selmon->pertag->prevtag = selmon->pertag->curtag;
        selmon->pertag->curtag = tmptag;
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

void moveleft(const Arg *arg) {
    tagtoleft(arg);
    viewtoleft(arg);
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

void viewtoleft(const Arg *arg) {
    int i;
    if (selmon->pertag->curtag == 1)
        return;
    if (__builtin_popcount(selmon->tagset[selmon->seltags] & TAGMASK) == 1 &&
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

void upkey(const Arg *arg) {
    if (!selmon->sel)
        return;

    if (&overviewlayout == selmon->lt[selmon->sellt]->arrange) {
        directionfocus(&((Arg){.ui = 0}));
        return;
    }

    if (NULL == selmon->lt[selmon->sellt]->arrange) {
        Client *c;
        c = selmon->sel;
        XSetWindowBorder(dpy, c->win,
                         borderscheme[SchemeBorderTileFocus].pixel);
        changesnap(c, 0);
        return;
    }
    focusstack(arg);
}

int unhideone() {
    if (selmon->sel && selmon->sel == selmon->overlay)
        return 0;
    Client *c;
    for (c = selmon->clients; c; c = c->next) {
        if (ISVISIBLE(c) && HIDDEN(c)) {
            show(c);
            focus(c);
            restack(selmon);
            return 1;
        }
    }
    return 0;
}

void downkey(const Arg *arg) {

    if (&overviewlayout == selmon->lt[selmon->sellt]->arrange) {
        directionfocus(&((Arg){.ui = 2}));
        return;
    }

    if (NULL == selmon->lt[selmon->sellt]->arrange) {
        if (!selmon->sel)
            return;
        // unmaximize
        changesnap(selmon->sel, 2);
        return;
    }
    focusstack(arg);
}

void spacetoggle(const Arg *arg) {
    if (NULL == selmon->lt[selmon->sellt]->arrange) {
        if (!selmon->sel)
            return;
        Client *c;
        c = selmon->sel;

        if (c->snapstatus) {
            resetsnap(c);
        } else {
            XSetWindowBorder(dpy, selmon->sel->win,
                             borderscheme[SchemeBorderTileFocus].pixel);
            savefloating(c);
            selmon->sel->snapstatus = 9;
            arrange(selmon);
        }
    } else {
        togglefloating(arg);
    }
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
            nextseltags =
                (curseltags << i) | (curseltags >> (LENGTH(tags) - 1 - i));
        else // right circular shift
            nextseltags =
                curseltags >> (-i) | (curseltags << (LENGTH(tags) - 1 + i));

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

    if (__builtin_popcount(selmon->tagset[selmon->seltags] & TAGMASK) == 1 &&
        selmon->tagset[selmon->seltags] & (TAGMASK >> 1)) {
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

// toggle overview like layout
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
        tempfullscreen();
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

Client *wintoclient(Window w) {
    Client *c;
    Monitor *m;

    for (m = mons; m; m = m->next)
        for (c = m->clients; c; c = c->next)
            if (c->win == w)
                return c;
    return NULL;
}

Client *wintosystrayicon(Window w) {
    Client *i = NULL;

    if (!showsystray || !w)
        return i;
    for (i = systray->icons; i && i->win != w; i = i->next)
        ;
    return i;
}

Monitor *wintomon(Window w) {
    int x, y;
    Client *c;
    Monitor *m;

    if (w == root && getrootptr(&x, &y))
        return recttomon(x, y, 1, 1);
    for (m = mons; m; m = m->next)
        if (w == m->barwin)
            return m;
    if ((c = wintoclient(w)))
        return c->mon;
    return selmon;
}

/* Selects for the view of the focused window. The list of tags */
/* to be displayed is matched to the focused window tag list. */
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
        if (selmon->pertag->curtag == 0) {
            lastview(NULL);
        }
        if (!selmon->scratchvisible) {
            togglescratchpad(NULL);
        }
    } else {
        view(&a);
    }
}

/* There's no way to check accesses to destroyed windows, thus those cases are
 * ignored (especially on UnmapNotify's). Other types of errors call Xlibs
 * default error handler, which may call exit. */
int xerror(Display *dpy, XErrorEvent *ee) {
    if (ee->error_code == BadWindow ||
        (ee->request_code == X_SetInputFocus && ee->error_code == BadMatch) ||
        (ee->request_code == X_PolyText8 && ee->error_code == BadDrawable) ||
        (ee->request_code == X_PolyFillRectangle &&
         ee->error_code == BadDrawable) ||
        (ee->request_code == X_PolySegment && ee->error_code == BadDrawable) ||
        (ee->request_code == X_ConfigureWindow && ee->error_code == BadMatch) ||
        (ee->request_code == X_GrabButton && ee->error_code == BadAccess) ||
        (ee->request_code == X_GrabKey && ee->error_code == BadAccess) ||
        (ee->request_code == X_CopyArea && ee->error_code == BadDrawable))
        return 0;
    fprintf(stderr, "instantwm: fatal error: request code=%d, error code=%d\n",
            ee->request_code, ee->error_code);
    return xerrorxlib(dpy, ee); /* may call exit */
}

int xerrordummy(Display *dpy, XErrorEvent *ee) { return 0; }

/* Startup Error handler to check if another window manager
 * is already running. */
int xerrorstart(Display *dpy, XErrorEvent *ee) {
    die("instantwm: another window manager is already running");
    return -1;
}

Monitor *systraytomon(Monitor *m) {
    Monitor *t;
    int i, n;
    if (!systraypinning) {
        if (!m)
            return selmon;
        return m == selmon ? m : NULL;
    }
    for (n = 1, t = mons; t && t->next; n++, t = t->next)
        ;
    for (i = 1, t = mons; t && t->next && i < systraypinning; i++, t = t->next)
        ;
    if (systraypinningfailfirst && n < systraypinning)
        return mons;
    return t;
}

void zoom(const Arg *arg) {
    Client *c = selmon->sel;

    if (!c)
        return;

    XRaiseWindow(dpy, c->win);

    if ((!selmon->lt[selmon->sellt]->arrange ||
         (selmon->sel && selmon->sel->isfloating)) ||
        (c == nexttiled(selmon->clients) &&
         (!c || !(c = nexttiled(c->next))))) {
        return;
    }
    pop(c);
}

void list_xresources() {

    int i, u, q;
    for (i = 0; i < LENGTH(schemehovertypes); i++) {
        for (q = 0; q < LENGTH(schemecolortypes); q++) {
            for (u = 0; u < LENGTH(schemewindowtypes); u++) {
                char propname[100] = "";
                snprintf(propname, sizeof(propname), "%s.%s.win.%s",
                         schemehovertypes[i].name, schemewindowtypes[u].name,
                         schemecolortypes[q].name);
                printf("instantwm.%s\n", propname);
            }
            for (u = 0; u < LENGTH(schemetagtypes); u++) {
                char propname[100] = "";
                snprintf(propname, sizeof(propname), "%s.%s.tag.%s",
                         schemehovertypes[i].name, schemetagtypes[u].name,
                         schemecolortypes[q].name);
                printf("instantwm.%s\n", propname);
            }
            for (u = 0; u < LENGTH(schemeclosetypes); u++) {
                char propname[100] = "";
                snprintf(propname, sizeof(propname), "%s.%s.close.%s",
                         schemehovertypes[i].name, schemeclosetypes[u].name,
                         schemecolortypes[q].name);
                printf("instantwm.%s\n", propname);
            }
        }
    }
    printf(
        "normal.border\nfocus.tile.border\nfocus.float.border\nsnap.border\n");
    printf("status.fg\nstatus.bg\nstatus.detail\n");
}

void resource_load(XrmDatabase db, char *name, enum resource_type rtype,
                   void *dst) {
    char *sdst = NULL;
    int *idst = NULL;
    float *fdst = NULL;

    sdst = dst;
    idst = dst;
    fdst = dst;

    char fullname[256];
    char *type;
    XrmValue ret;

    snprintf(fullname, sizeof(fullname), "%s.%s", "instantwm", name);
    fullname[sizeof(fullname) - 1] = '\0';

    XrmGetResource(db, fullname, "*", &type, &ret);
    if (!(ret.addr == NULL || strncmp("String", type, 64))) {
        switch (rtype) {
        case STRING:
            strcpy(sdst, ret.addr);
            break;
        case INTEGER:
            *idst = strtoul(ret.addr, NULL, 10);
            break;
        case FLOAT:
            *fdst = strtof(ret.addr, NULL);
            break;
        }
    }
}

int main(int argc, char *argv[]) {
    if (argc == 2) {
        if (!strcmp("-V", argv[1]) || !strcmp("--version", argv[1])) {
            puts("instantwm-" VERSION "\n");
            return EXIT_SUCCESS;
        } else if (!strcmp("-X", argv[1]) || !strcmp("--xresources", argv[1])) {
            list_xresources();
            return EXIT_SUCCESS;
        } else {
            die("usage: instantwm [-VX]");
        }
    } else if (argc != 1)
        die("usage: instantwm [-VX]");
    if (!setlocale(LC_CTYPE, "") || !XSupportsLocale())
        fputs("warning: no locale support\n", stderr);
    if (!(dpy = XOpenDisplay(NULL)))
        die("instantwm: cannot open display");
    checkotherwm();
    XrmInitialize();
    load_xresources();
    setup();
#ifdef __OpenBSD__
    if (pledge("stdio rpath proc exec", NULL) == -1)
        die("pledge");
#endif /* __OpenBSD__ */
    scan();
    runAutostart();
    run();
    cleanup();
    XCloseDisplay(dpy);
    return EXIT_SUCCESS;
}

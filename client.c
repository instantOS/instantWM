/* See LICENSE file for copyright and license details. */

#include "client.h"
#include "animation.h"
#include "bar.h"
#include "globals.h"
#include "instantwm.h"
#include "layouts.h"
#include "monitors.h"
#include "mouse.h"
#include "push.h"
#include "scratchpad.h"
#include "util.h"
#include "xresources.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* External declarations not covered by headers */
extern Client *lastclient;
extern const char broken[];

/* Globals moved to client.c */
Client *animclient;

void attach(Client *c) {
    c->next = c->mon->clients;
    c->mon->clients = c;
}

void attachstack(Client *c) {
    c->snext = c->mon->stack;
    c->mon->stack = c;
}

void detach(Client *c) {
    Client **tc;

    for (tc = &c->mon->clients; *tc && *tc != c; tc = &(*tc)->next) {
        ;
    }
    *tc = c->next;
}

void detachstack(Client *c) {
    Client **tc;
    Client *t;

    for (tc = &c->mon->stack; *tc && *tc != c; tc = &(*tc)->snext) {
        ;
    }
    *tc = c->snext;

    if (c == c->mon->sel) {
        for (t = c->mon->stack; t && !ISVISIBLE(t); t = t->snext) {
            ;
        }
        c->mon->sel = t;
    }
}

Client *nexttiled(Client *c) {
    for (; c && (c->isfloating || !ISVISIBLE(c) || HIDDEN(c)); c = c->next) {
        ;
    }
    return c;
}

void pop(Client *c) {
    detach(c);
    attach(c);
    focus(c);
    arrange(c->mon);
}

Client *wintoclient(Window w) {
    Client *c;
    Monitor *m;

    for (m = mons; m; m = m->next) {
        for (c = m->clients; c; c = c->next) {
            if (c->win == w) {
                return c;
            }
        }
    }
    return NULL;
}

void setclientstate(Client *c, long state) {
    long data[] = {state, None};

    XChangeProperty(dpy, c->win, wmatom[WMState], wmatom[WMState], 32,
                    PropModeReplace, (unsigned char *)data, 2);
}

void setclienttagprop(Client *c) {
    long data[] = {(long)c->tags, (long)c->mon->num};
    XChangeProperty(dpy, c->win, netatom[NetClientInfo], XA_CARDINAL, 32,
                    PropModeReplace, (unsigned char *)data, 2);
}

int sendevent(Window w, Atom proto, int mask, long d0, long d1, long d2,
              long d3, long d4) {
    int n;
    Atom *protocols;
    Atom mt;
    int exists = 0;
    XEvent ev;

    if (proto == wmatom[WMTakeFocus] || proto == wmatom[WMDelete]) {
        mt = wmatom[WMProtocols];
        if (XGetWMProtocols(dpy, w, &protocols, &n)) {
            while (!exists && n--) {
                exists = protocols[n] == proto;
            }
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
    ce.border_width = c->border_width;
    ce.above = None;
    ce.override_redirect = False;
    XSendEvent(dpy, c->win, False, StructureNotifyMask, (XEvent *)&ce);
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

void unfocus(Client *c, int setfocus) {
    if (!c) {
        return;
    }
    lastclient = c;
    grabbuttons(c, 0);
    XSetWindowBorder(dpy, c->win, borderscheme[SchemeBorderNormal].pixel);
    if (setfocus) {
        XSetInputFocus(dpy, root, RevertToPointerRoot, CurrentTime);
        XDeleteProperty(dpy, root, netatom[NetActiveWindow]);
    }
}

void showhide(Client *c) {
    if (!c) {
        return;
    }
    if (ISVISIBLE(c)) {
        /* show clients top down */
        XMoveWindow(dpy, c->win, c->x, c->y);
        if ((!c->mon->lt[c->mon->sellt]->arrange || c->isfloating) &&
            (!c->is_fullscreen || c->isfakefullscreen)) {
            resize(c, c->x, c->y, c->w, c->h, 0);
        }
        showhide(c->snext);
    } else {
        /* hide clients bottom up */
        showhide(c->snext);
        XMoveWindow(dpy, c->win, WIDTH(c) * -2, c->y);
    }
}

void show(Client *c) {
    int x;
    int y;
    int w;
    int h;
    if (!c || !HIDDEN(c)) {
        return;
    }

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

void hide(Client *c) {
    if (!c || HIDDEN(c)) {
        return;
    }

    int x;
    int y;
    int wi;
    int h;
    x = c->x;
    y = c->y;
    wi = c->w;
    h = c->h;

    if (animated) {
        animateclient(c, c->x, bh - c->h + 40, 0, 0, 10, 0);
    }

    Window w = c->win;
    static XWindowAttributes ra;
    static XWindowAttributes ca;

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

void resize(Client *c, int x, int y, int w, int h, int interact) {
    if (applysizehints(c, &x, &y, &w, &h, interact) ||
        selmon->clientcount == 1) {
        resizeclient(c, x, y, w, h);
    }
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
    wc.border_width = c->border_width;

    XConfigureWindow(dpy, c->win,
                     CWX | CWY | CWWidth | CWHeight | CWBorderWidth, &wc);
    configure(c);
    /* Use XFlush to avoid blocking round-trips */
    XFlush(dpy);
}

void updatetitle(Client *c) {
    if (!gettextprop(c->win, netatom[NetWMName], c->name, sizeof c->name)) {
        gettextprop(c->win, XA_WM_NAME, c->name, sizeof c->name);
    }
    if (c->name[0] == '\0') {
        strcpy(c->name, broken);
    }
}

/* Moved functions */

void desktopset() {
    Client *c = selmon->sel;
    c->isfloating = 0;
    arrange(c->mon);
    resize(c, 0, bh, drw->w, drw->h - bh, 0);
    unmanage(c, 0);
    restack(selmon);
}

void applyrules(Client *c) {
    const char *class;
    const char *instance;
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
        case SpecialFloat:
            c->isfloating = 1;
            break;
        }
        specialnext = SpecialNone;
    } else {
        unsigned int i;
        const Rule *r;
        for (i = 0; i < rules_len; i++) {
            r = &rules[i];
            if ((!r->title || strstr(c->name, r->title)) &&
                (!r->class || strstr(class, r->class)) &&
                (!r->instance || strstr(instance, r->instance))) {
                if (r->class && strstr(r->class, "Onboard") != NULL) {
                    c->issticky = 1;
                }

                switch (r->isfloating) {
                case RuleFloatCenter:
                    selmon->sel = c;
                    c->isfloating = 1;
                    center_window(NULL);
                    break;
                case RuleFloatFullscreen:
                    /* fullscreen overlay */
                    selmon->sel = c;
                    c->isfloating = 1;
                    c->w = c->mon->mw;
                    c->h = c->mon->wh - (selmon->showbar ? bh : 0);
                    if (selmon->showbar) {
                        c->y = selmon->my + bh;
                    }
                    c->x = selmon->mx;
                    break;
                case RuleScratchpad:
                    selmon->sel = c;
                    c->tags = SCRATCHPAD_MASK;
                    selmon->scratchvisible = 1;
                    c->issticky = 1;
                    c->isfloating = 1;
                    selmon->activescratchpad = c;
                    center_window(NULL);
                    break;
                case RuleFloat:
                    c->isfloating = 1;
                    c->y = c->mon->my + (selmon->showbar ? bh : 0);
                    break;
                case RuleTiled:
                    c->isfloating = 0;
                    break;
                }

                c->tags |= r->tags;
                for (m = mons; m && m->num != r->monitor; m = m->next) {
                    ;
                }
                if (m) {
                    c->mon = m;
                }
            }
        }
    }
    if (ch.res_class) {
        XFree(ch.res_class);
    }
    if (ch.res_name) {
        XFree(ch.res_name);
    }
    c->tags =
        c->tags & tagmask ? c->tags & tagmask : c->mon->tagset[c->mon->seltags];
}

int applysizehints(Client *c, int *x, int *y, int *w, int *h, int interact) {
    Monitor *m = c->mon;

    /* set minimum possible */
    *w = MAX(1, *w);
    *h = MAX(1, *h);
    if (interact) {
        if (*x > sw) {
            *x = sw - WIDTH(c);
        }
        if (*y > sh) {
            *y = sh - HEIGHT(c);
        }
        if (*x + *w + 2 * c->border_width < 0) {
            *x = 0;
        }
        if (*y + *h + 2 * c->border_width < 0) {
            *y = 0;
        }
    } else {
        if (*x >= m->wx + m->ww) {
            *x = m->wx + m->ww - WIDTH(c);
        }
        if (*y >= m->wy + m->wh) {
            *y = m->wy + m->wh - HEIGHT(c);
        }
        if (*x + *w + 2 * c->border_width <= m->wx) {
            *x = m->wx;
        }
        if (*y + *h + 2 * c->border_width <= m->wy) {
            *y = m->wy;
        }
    }
    if (*h < bh) {
        *h = bh;
    }
    if (*w < bh) {
        *w = bh;
    }
    if (resizehints || c->isfloating || !c->mon->lt[c->mon->sellt]->arrange) {
        if (!c->hintsvalid) {
            updatesizehints(c);
        }
        /* see last two sentences in ICCCM 4.1.2.3 */
        int baseismin = c->basew == c->minw && c->baseh == c->minh;
        if (!baseismin) { /* temporarily remove base dimensions */
            *w -= c->basew;
            *h -= c->baseh;
        }
        /* adjust for aspect limits */
        if (c->mina > 0 && c->maxa > 0) {
            if (c->maxa < (float)*w / *h) {
                *w = *h * c->maxa + 0.5;
            } else if (c->mina < (float)*h / *w) {
                *h = *w * c->mina + 0.5;
            }
        }
        if (baseismin) { /* increment calculation requires this */
            *w -= c->basew;
            *h -= c->baseh;
        }
        /* adjust for increment value */
        if (c->incw) {
            *w -= *w % c->incw;
        }
        if (c->inch) {
            *h -= *h % c->inch;
        }
        /* restore base dimensions */
        *w = MAX(*w + c->basew, c->minw);
        *h = MAX(*h + c->baseh, c->minh);
        if (c->maxw) {
            *w = MIN(*w, c->maxw);
        }
        if (c->maxh) {
            *h = MIN(*h, c->maxh);
        }
    }
    return *x != c->x || *y != c->y || *w != c->w || *h != c->h;
}

// close selected client
void killclient(const Arg *arg) {
    if (!selmon->sel || selmon->sel->islocked) {
        return;
    }
    if (animated && selmon->sel != animclient && !selmon->sel->is_fullscreen) {
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

    if (!c || c->islocked) {
        return;
    }

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
    Client *c;
    Client *t = NULL;
    Window trans = None;
    XWindowChanges wc;

    c = ecalloc(1, sizeof(Client));
    c->win = w;
    /* geometry */
    c->x = c->oldx = wa->x;
    c->y = c->oldy = wa->y;
    c->w = c->oldw = wa->width;
    c->h = c->oldh = wa->height;
    c->old_border_width = wa->border_width;

    updatetitle(c);
    if (XGetTransientForHint(dpy, w, &trans) && (t = wintoclient(trans))) {
        c->mon = t->mon;
        c->tags = t->tags;
    } else {
        c->mon = selmon;
        applyrules(c);
    }

    if (c->x + WIDTH(c) > c->mon->wx + c->mon->ww) {
        c->x = c->mon->wx + c->mon->ww - WIDTH(c);
    }
    if (c->y + HEIGHT(c) > c->mon->wy + c->mon->wh) {
        c->y = c->mon->wy + c->mon->wh - HEIGHT(c);
    }
    c->x = MAX(c->x, c->mon->wx);
    /* only fix client y-offset, if the client center might cover the bar */
    c->y = MAX(c->y, c->mon->wy);
    c->border_width = borderpx;

    if (!c->isfloating && &monocle == c->mon->lt[c->mon->sellt]->arrange &&
        c->w > c->mon->mw - 30 && c->h > (c->mon->mh - 30 - bh)) {
        wc.border_width = 0;
    } else {
        wc.border_width = c->border_width;
    }

    XConfigureWindow(dpy, w, CWBorderWidth, &wc);
    XSetWindowBorder(dpy, w, borderscheme[SchemeBorderNormal].pixel);
    configure(c); /* propagates border_width, if size doesn't change */
    updatewindowtype(c);
    updatesizehints(c);
    updatewmhints(c);

    {
        int format;
        unsigned long *data;
        unsigned long n;
        unsigned long extra;
        Monitor *m;
        Atom atom;
        if (XGetWindowProperty(dpy, c->win, netatom[NetClientInfo], 0L, 2L,
                               False, XA_CARDINAL, &atom, &format, &n, &extra,
                               (unsigned char **)&data) == Success &&
            n == 2) {
            c->tags = *data;
            for (m = mons; m; m = m->next) {
                if (m->num == *(data + 1)) {
                    c->mon = m;
                    break;
                }
            }
        }
        if (n > 0) {
            XFree(data);
        }
    }
    setclienttagprop(c);

    updatemotifhints(c);

    c->saved_float_x = c->x;
    c->saved_float_y = c->y = c->y >= c->mon->my ? c->y : c->y + c->mon->my;
    c->saved_float_width = c->w;
    c->saved_float_height = c->h;
    XSelectInput(dpy, w,
                 EnterWindowMask | FocusChangeMask | PropertyChangeMask |
                     StructureNotifyMask);
    grabbuttons(c, 0);
    if (!c->isfloating) {
        c->isfloating = c->oldstate = trans != None || c->isfixed;
    }
    if (c->isfloating) {
        XRaiseWindow(dpy, c->win);
    }
    attach(c);
    attachstack(c);
    XChangeProperty(dpy, root, netatom[NetClientList], XA_WINDOW, 32,
                    PropModeAppend, (unsigned char *)&(c->win), 1);
    XMoveResizeWindow(dpy, c->win, c->x + 2 * sw, c->y, c->w,
                      c->h); /* some windows require this */
    if (!HIDDEN(c)) {
        setclientstate(c, NormalState);
    }
    if (c->mon == selmon) {
        unfocus(selmon->sel, 0);
    }
    c->mon->sel = c;
    arrange(c->mon);
    if (!HIDDEN(c)) {
        XMapWindow(dpy, c->win);
    }
    focus(NULL);

    if (animated && !c->is_fullscreen) {
        resizeclient(c, c->x, c->y - 70, c->w, c->h);
        animateclient(c, c->x, c->y + 70, 0, 0, 7, 0);
        if (NULL == c->mon->lt[selmon->sellt]->arrange) {
            XRaiseWindow(dpy, c->win);
        } else {
            if (c->w > selmon->mw - 30 || c->h > selmon->mh - 30) {
                arrange(selmon);
            }
        }
    }
}

void shutkill(const Arg *arg) {
    if (!selmon->clients) {
        spawn(&((Arg){.v = instantshutdowncmd}));
    } else {
        killclient(arg);
    }
}

void setfullscreen(Client *c, int fullscreen) {
    if (fullscreen && !c->is_fullscreen) {
        XChangeProperty(dpy, c->win, netatom[NetWMState], XA_ATOM, 32,
                        PropModeReplace,
                        (unsigned char *)&netatom[NetWMFullscreen], 1);
        c->is_fullscreen = 1;

        c->oldstate = c->isfloating;
        savebw(c);
        if (!c->isfakefullscreen) {
            c->border_width = 0;
            if (!c->isfloating) {
                animateclient(c, c->mon->mx, c->mon->my, c->mon->mw, c->mon->mh,
                              10, 0);
            }
            resizeclient(c, c->mon->mx, c->mon->my, c->mon->mw, c->mon->mh);
            XRaiseWindow(dpy, c->win);
        }
        c->isfloating = 1;

    } else if (!fullscreen && c->is_fullscreen) {
        XChangeProperty(dpy, c->win, netatom[NetWMState], XA_ATOM, 32,
                        PropModeReplace, (unsigned char *)0, 0);
        c->is_fullscreen = 0;

        c->isfloating = c->oldstate;
        restore_border_width(c);
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

void togglefakefullscreen(const Arg *arg) {
    if (selmon->sel->is_fullscreen) {
        if (selmon->sel->isfakefullscreen) {
            resizeclient(selmon->sel, selmon->mx + borderpx,
                         selmon->my + borderpx, selmon->mw - 2 * borderpx,
                         selmon->mh - 2 * borderpx);
            XRaiseWindow(dpy, selmon->sel->win);
        } else {
            selmon->sel->border_width = selmon->sel->old_border_width;
        }
    }

    selmon->sel->isfakefullscreen = !selmon->sel->isfakefullscreen;
}

// minimize window
void hide_window(const Arg *arg) {
    if (!selmon->sel) {
        return;
    }
    Client *c = selmon->sel;
    if (HIDDEN(c)) {
        return;
    }
    hide(c);
}

// fixes drawing issues with wine games
void redrawwin(const Arg *arg) {
    int tmpanimated = 0;
    if (!selmon->sel) {
        return;
    }
    Client *c = selmon->sel;
    if (HIDDEN(c)) {
        return;
    }
    if (animated) {
        tmpanimated = 1;
        animated = 0;
    }

    hide(c);
    show(c);
    if (tmpanimated) {
        animated = 1;
    }
}

void unhide_all(const Arg *arg) {

    Client *c;
    for (c = selmon->clients; c; c = c->next) {
        if (ISVISIBLE(c) && HIDDEN(c)) {
            show(c);
        }
    }
    focus(c);
    restack(selmon);
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

    if (c == m->activescratchpad) {
        m->activescratchpad = NULL;
    }

    detach(c);
    detachstack(c);
    if (!destroyed) {
        wc.border_width = c->old_border_width;
        XGrabServer(dpy); /* avoid race conditions */
        XSetErrorHandler(xerrordummy);
        XSelectInput(dpy, c->win, NoEventMask);
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

// fix issues with custom window borders
void updatemotifhints(Client *c) {
    Atom real;
    int format;
    unsigned char *p = NULL;
    unsigned long n;
    unsigned long extra;
    unsigned long *motif;
    int width;
    int height;

    if (!decorhints) {
        return;
    }

    if (XGetWindowProperty(dpy, c->win, motifatom, 0L, 5L, False, motifatom,
                           &real, &format, &n, &extra, &p) == Success &&
        p != NULL) {
        motif = (unsigned long *)p;
        if (motif[MWM_HINTS_FLAGS_FIELD] & MWM_HINTS_DECORATIONS) {
            width = WIDTH(c);
            height = HEIGHT(c);

            if (motif[MWM_HINTS_DECORATIONS_FIELD] & MWM_DECOR_ALL ||
                motif[MWM_HINTS_DECORATIONS_FIELD] & MWM_DECOR_BORDER ||
                motif[MWM_HINTS_DECORATIONS_FIELD] & MWM_DECOR_TITLE) {
                c->border_width = c->old_border_width = borderpx;
            } else {
                c->border_width = c->old_border_width = 0;
            }

            resize(c, c->x, c->y, width - (2 * c->border_width),
                   height - (2 * c->border_width), 0);
        }
        XFree(p);
    }
}

void updatesizehints(Client *c) {
    long msize;
    XSizeHints size;

    if (!XGetWMNormalHints(dpy, c->win, &size, &msize)) {
        /* size is uninitialized, ensure that size.flags aren't used */
        size.flags = PSize;
    }
    if (size.flags & PBaseSize) {
        c->basew = size.base_width;
        c->baseh = size.base_height;
    } else if (size.flags & PMinSize) {
        c->basew = size.min_width;
        c->baseh = size.min_height;
    } else {
        c->basew = c->baseh = 0;
    }
    if (size.flags & PResizeInc) {
        c->incw = size.width_inc;
        c->inch = size.height_inc;
    } else {
        c->incw = c->inch = 0;
    }
    if (size.flags & PMaxSize) {
        c->maxw = size.max_width;
        c->maxh = size.max_height;
    } else {
        c->maxw = c->maxh = 0;
    }
    if (size.flags & PMinSize) {
        c->minw = size.min_width;
        c->minh = size.min_height;
    } else if (size.flags & PBaseSize) {
        c->minw = size.base_width;
        c->minh = size.base_height;
    } else {
        c->minw = c->minh = 0;
    }
    if (size.flags & PAspect) {
        c->mina = (float)size.min_aspect.y / size.min_aspect.x;
        c->maxa = (float)size.max_aspect.x / size.max_aspect.y;
    } else {
        c->maxa = c->mina = 0.0;
    }
    c->isfixed =
        (c->maxw && c->maxh && c->maxw == c->minw && c->maxh == c->minh);
    c->hintsvalid = 1;
}

void updatewindowtype(Client *c) {
    Atom state = getatomprop(c, netatom[NetWMState]);
    Atom wtype = getatomprop(c, netatom[NetWMWindowType]);

    if (state == netatom[NetWMFullscreen]) {
        setfullscreen(c, 1);
    }
    if (wtype == netatom[NetWMWindowTypeDialog]) {
        c->isfloating = 1;
    }
}

void updatewmhints(Client *c) {
    XWMHints *wmh;

    if ((wmh = XGetWMHints(dpy, c->win))) {
        if (c == selmon->sel && wmh->flags & XUrgencyHint) {
            wmh->flags &= ~XUrgencyHint;
            XSetWMHints(dpy, c->win, wmh);
        } else {
            c->isurgent = (wmh->flags & XUrgencyHint) ? 1 : 0;
        }
        if (wmh->flags & InputHint) {
            c->neverfocus = !wmh->input;
        } else {
            c->neverfocus = 0;
        }
        XFree(wmh);
    }
}

int unhideone() {
    if (selmon->sel && selmon->sel == selmon->overlay) {
        return 0;
    }
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

void zoom(const Arg *arg) {
    Client *c = selmon->sel;

    if (!c) {
        return;
    }

    XRaiseWindow(dpy, c->win);

    if ((!tiling_layout_func(selmon) ||
         (selmon->sel && selmon->sel->isfloating)) ||
        (c == nexttiled(selmon->clients) &&
         (!c || !(c = nexttiled(c->next))))) {
        return;
    }
    pop(c);
}

void seturgent(Client *c, int urg) {
    XWMHints *wmh;

    c->isurgent = urg;
    if (!(wmh = XGetWMHints(dpy, c->win))) {
        return;
    }
    wmh->flags =
        urg ? (wmh->flags | XUrgencyHint) : (wmh->flags & ~XUrgencyHint);
    XSetWMHints(dpy, c->win, wmh);
    XFree(wmh);
}

void updateclientlist(void) {
    Client *c;
    Monitor *m;

    XDeleteProperty(dpy, root, netatom[NetClientList]);
    for (m = mons; m; m = m->next) {
        for (c = m->clients; c; c = c->next) {
            XChangeProperty(dpy, root, netatom[NetClientList], XA_WINDOW, 32,
                            PropModeAppend, (unsigned char *)&(c->win), 1);
        }
    }
}

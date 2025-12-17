/* See LICENSE file for copyright and license details. */

#include "events.h"
#include "animation.h"
#include "bar.h"
#include "client.h"
#include "floating.h"
#include "focus.h"
#include "layouts.h"
#include "monitors.h"
#include "mouse.h"
#include "overlay.h"
#include "scratchpad.h"
#include "systray.h"
#include "tags.h"
#include "util.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* extern variables from instantwm.c */
extern Display *dpy;
extern Drw *drw;
extern Monitor *selmon;
extern Monitor *mons;
extern Window root;
extern int bh;
extern int lrpad;
extern int animated;
extern int tagwidth;
extern int altcursor;
extern int focusfollowsmouse;
extern int focusfollowsfloatmouse;
extern Cur *cursor[CurLast];
extern Clr *borderscheme;
extern Atom wmatom[];
extern Atom netatom[];
extern Atom xatom[];
extern Atom motifatom;
extern Systray *systray;
extern int statuswidth;

/* config.h values (non-static) */
extern const int showsystray;
extern const unsigned int startmenusize;
extern char tags[][16];
extern const Layout layouts[];
extern int numtags;

/* extern functions from instantwm.c */
extern void grabkeys(void);
extern void updatebarpos(Monitor *m);
extern void updatenumlockmask(void);
extern void updateclientlist(void);
extern void manage(Window w, XWindowAttributes *wa);
extern void unmanage(Client *c, int destroyed);
extern void updatesizehints(Client *c);
extern void updatewindowtype(Client *c);
extern void updatewmhints(Client *c);
extern void updatemotifhints(Client *c);
extern void resizebarwin(Monitor *m);
extern void resetcursor(void);
extern int get_blw(Monitor *m);
extern long getstate(Window w);
extern int getrootptr(int *x, int *y);
extern Client *getcursorclient(void);

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
            extern Clr *statusscheme;
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
        extern void setfullscreen(Client * c, int fullscreen);
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
            showoverlay(NULL);
        } else if (c->tags == SCRATCHPAD_MASK) {
            selmon = c->mon;
            togglescratchpad(NULL);
        } else {
            if (HIDDEN(c))
                show(c);
            for (i = 0; i < numtags && !((1 << i) & c->tags); i++)
                ;
            if (i < numtags) {
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

void configurenotify(XEvent *e) {
    Monitor *m;
    Client *c;
    XConfigureEvent *ev = &e->xconfigure;
    extern int sw, sh;

    /* TODO: updategeom handling sucks, needs to be simplified */
    if (ev->window == root) {
        int dirty = (sw != ev->width || sh != ev->height);
        sw = ev->width;
        sh = ev->height;
        extern int updategeom(void);
        extern void updatebars(void);
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

void configurerequest(XEvent *e) {
    Client *c;
    Monitor *m;
    XConfigureRequestEvent *ev = &e->xconfigurerequest;
    XWindowChanges wc;

    if ((c = wintoclient(ev->window))) {
        if (ev->value_mask & CWBorderWidth)
            c->bw = ev->border_width;
        else if (c->isfloating || !tiling_layout_func(selmon)) {
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

void enternotify(XEvent *e) {
    Client *c;
    Monitor *m;
    XCrossingEvent *ev = &e->xcrossing;
    int resizeexit = 0;
    static int barleavestatus = 0;

    /* deactivate area at the top to prevent overlay gesture from glitching out
     */
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
        (selmon->sel->isfloating || !tiling_layout_func(selmon)) &&
        c != selmon->sel &&
        (ev->window == root || visible(c) || ISVISIBLE(c) ||
         selmon->sel->issticky)) {
        resizeexit = hoverresizemouse(NULL);
        if (focusfollowsfloatmouse) {
            if (resizeexit) // If resize was performed, don't change focus
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

/* there are some broken focus acquiring clients needing extra handling */
void focusin(XEvent *e) {
    XFocusChangeEvent *ev = &e->xfocus;

    if (selmon->sel && ev->window != selmon->sel->win)
        setfocus(selmon->sel);
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

    if (!XGetWindowAttributes(dpy, ev->window, &wa) || wa.override_redirect)
        return;

    if (!wintoclient(ev->window))
        manage(ev->window, &wa);
}

/* Helper: handle hover near floating window for resize cursor */
static int handlefloatingresizehover(Monitor *m) {
    if (!(selmon->sel &&
          (selmon->sel->isfloating || NULL == tiling_layout_func(selmon))))
        return 0;

    Client *c;
    int tilefound = 0;
    for (c = m->clients; c; c = c->next) {
        if (ISVISIBLE(c) &&
            !(c->isfloating || NULL == tiling_layout_func(selmon))) {
            tilefound = 1;
            break;
        }
    }
    if (tilefound)
        return 0;

    if (isinresizeborder()) {
        if (altcursor != AltCurResize) {
            XDefineCursor(dpy, root, cursor[CurResize]->cursor);
            altcursor = AltCurResize;
        }
        Client *newc = getcursorclient();
        if (newc && newc != selmon->sel)
            focus(newc);
        return 1;
    } else if (altcursor == AltCurResize) {
        resetcursor();
    }
    return 0;
}

/* Helper: handle sidebar slider cursor */
static int handlesidebarhover(XMotionEvent *ev) {
    if (ev->x_root > selmon->mx + selmon->mw - 50) {
        if (altcursor == AltCurNone && ev->y_root > bh + 60) {
            altcursor = AltCurSidebar;
            XDefineCursor(dpy, root, cursor[CurVert]->cursor);
        }
        return 1;
    } else if (altcursor == AltCurSidebar) {
        altcursor = AltCurNone;
        XUndefineCursor(dpy, root);
        XDefineCursor(dpy, root, cursor[CurNormal]->cursor);
        return 1;
    }
    return 0;
}

/* Helper: handle overlay corner gesture */
static int handleoverlaygesture(XMotionEvent *ev) {
    if (ev->y_root == selmon->my &&
        ev->x_root >= selmon->mx + selmon->ww - 20 - getsystraywidth()) {
        if (selmon->gesture != 11) {
            selmon->gesture = 11;
            setoverlay(NULL);
        }
        return 1;
    } else if (selmon->gesture == 11 &&
               ev->x_root >= selmon->mx + selmon->ww - 24 - getsystraywidth()) {
        selmon->gesture = GestureNone;
        return 1;
    }
    return 0;
}

/* Helper: handle tag bar hover */
static void handletagbarhover(XMotionEvent *ev) {
    extern int lrpad;
    if (selmon->hoverclient)
        selmon->hoverclient = NULL;

    if (ev->x_root < selmon->mx + tagwidth && !selmon->showtags) {
        if (ev->x_root < selmon->mx + startmenusize) {
            selmon->gesture = GestureStartMenu;
            drawbar(selmon);
        } else {
            int i = 0;
            int x = selmon->mx + startmenusize;
            do {
                x += TEXTW(tags[i]);
            } while (ev->x_root >= x && ++i < 8);

            if (i != selmon->gesture - 1) {
                selmon->gesture = i + 1;
                drawbar(selmon);
            }
        }
    } else {
        resetbar();
    }
}

/* Helper: handle title bar hover */
static void handletitlebarhover(XMotionEvent *ev) {
    /* hover over close button */
    if (ev->x_root > selmon->activeoffset &&
        ev->x_root < (selmon->activeoffset + 32)) {
        if (selmon->gesture != 12) {
            selmon->gesture = 12;
            drawbar(selmon);
        }
    } else if (selmon->gesture == 12) {
        selmon->gesture = GestureNone;
        drawbar(selmon);
    } else {
        /* hover over resize widget on title bar */
        double titlewidth = (1.0 / (double)selmon->bt) * selmon->btw;
        int resizeStart = selmon->activeoffset + titlewidth - 30;
        int resizeEnd = selmon->activeoffset + titlewidth;

        if (altcursor == AltCurNone) {
            if (ev->x_root > resizeStart && ev->x_root < resizeEnd) {
                XDefineCursor(dpy, root, cursor[CurResize]->cursor);
                altcursor = AltCurResize;
            }
        } else if (ev->x_root < resizeStart || ev->x_root > resizeEnd) {
            XDefineCursor(dpy, root, cursor[CurNormal]->cursor);
            altcursor = AltCurNone;
        }
    }

    /* indicator when hovering over clients */
    if (selmon->stack) {
        int x = selmon->mx + tagwidth + 60;
        Client *c = selmon->clients;
        do {
            if (!ISVISIBLE(c))
                continue;
            else
                x += (1.0 / (double)selmon->bt) * selmon->btw;
        } while (ev->x_root > x && (c = c->next));

        if (c && c != selmon->hoverclient) {
            selmon->hoverclient = c;
            selmon->gesture = GestureNone;
            drawbar(selmon);
        }
    }
}

void motionnotify(XEvent *e) {
    Monitor *m;
    XMotionEvent *ev = &e->xmotion;

    if (ev->window != root)
        return;

    if (!tagwidth)
        tagwidth = gettagwidth();

    /* detect mouse hovering over other monitor */
    m = recttomon(ev->x_root, ev->y_root, 1, 1);
    if (m && m != selmon && focusfollowsmouse) {
        unfocus(selmon->sel, 1);
        selmon = m;
        focus(NULL);
        return;
    }

    /* hover below bar (in desktop area) */
    if (ev->y_root >= selmon->my + bh - 3) {
        if (handlefloatingresizehover(m))
            return;
        if (handlesidebarhover(ev))
            return;
        resetbar();
        if (altcursor == AltCurSidebar)
            resetcursor();
        return;
    } else {
        /* barleavestatus handled in enternotify */
    }

    /* hover in bar area */
    if (handleoverlaygesture(ev))
        return;

    /* cursor is to the left of window titles (tags area) */
    if (ev->x_root < selmon->mx + tagwidth + 60) {
        handletagbarhover(ev);
    } else if (selmon->sel &&
               ev->x_root < selmon->mx + 60 + tagwidth + selmon->btw) {
        /* cursor is on window titles */
        handletitlebarhover(ev);
    } else {
        resetbar();
    }
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
    extern int xcommand(void);
    extern void updatestatus(void);
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

void resizerequest(XEvent *e) {
    XResizeRequestEvent *ev = &e->xresizerequest;
    Client *i;

    if ((i = wintosystrayicon(ev->window))) {
        updatesystrayicongeom(i, ev->width, ev->height);
        resizebarwin(selmon);
        updatesystray();
    }
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

void leavenotify(XEvent *e) {
    XCrossingEvent *ev = &e->xcrossing;
    Monitor *m;

    if ((m = wintomon(ev->window)) && ev->window == m->barwin) {
        resetbar();
    }
}

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

#include "globals.h"

/* Extern functions are defined in instantwm.h or other headers,
 * keeping only those that might be missing or specific here if any.
 * Most function prototypes should be in headers. */

/* extern functions from instantwm.c */
extern void grabkeys(void);
extern void updatenumlockmask(void);
extern void updateclientlist(void);
extern void updatesizehints(Client *c);
extern void updatewindowtype(Client *c);
extern void updatewmhints(Client *c);
extern void updatemotifhints(Client *c);
extern void resetcursor(void);
extern long getstate(Window w);
extern int getrootptr(int *x, int *y);
extern Client *getcursorclient(void);

/* Helper: Handle systray dock request message */
static void handle_systray_dock_request(XClientMessageEvent *cme) {
    XWindowAttributes wa;
    XSetWindowAttributes swa;
    Client *c;

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
    c->old_border_width = wa.border_width;
    c->border_width = 0;
    c->isfloating = True;
    /* reuse tags field as mapped status */
    c->tags = 1;
    updatesizehints(c);
    updatesystrayicongeom(c, wa.width, wa.height);
    XAddToSaveSet(dpy, c->win);
    XSelectInput(dpy, c->win,
                 StructureNotifyMask | PropertyChangeMask | ResizeRedirectMask);
    XReparentWindow(dpy, c->win, systray->win, 0, 0);
    /* use parents background color */
    extern Clr *statusscheme;
    swa.background_pixel = statusscheme[ColBg].pixel;
    XChangeWindowAttributes(dpy, c->win, CWBackPixel, &swa);
    sendevent(c->win, netatom[Xembed], StructureNotifyMask, CurrentTime,
              XEMBED_EMBEDDED_NOTIFY, 0, systray->win, XEMBED_EMBEDDED_VERSION);
    /* FIXME not sure if I have to send these events, too */
    sendevent(c->win, netatom[Xembed], StructureNotifyMask, CurrentTime,
              XEMBED_FOCUS_IN, 0, systray->win, XEMBED_EMBEDDED_VERSION);
    sendevent(c->win, netatom[Xembed], StructureNotifyMask, CurrentTime,
              XEMBED_WINDOW_ACTIVATE, 0, systray->win, XEMBED_EMBEDDED_VERSION);
    sendevent(c->win, netatom[Xembed], StructureNotifyMask, CurrentTime,
              XEMBED_MODALITY_ON, 0, systray->win, XEMBED_EMBEDDED_VERSION);
    XSync(dpy, False);
    resizebarwin(selmon);
    updatesystray();
    setclientstate(c, NormalState);
}

/* Helper: Handle _NET_WM_STATE message (fullscreen toggle) */
static void handle_netWMstate(Client *c, XClientMessageEvent *cme) {
    extern void setfullscreen(Client * c, int fullscreen);
    if (cme->data.l[1] == netatom[NetWMFullscreen] ||
        cme->data.l[2] == netatom[NetWMFullscreen])
        setfullscreen(c, (cme->data.l[0] == 1 /* _NET_WM_STATE_ADD    */
                          || (cme->data.l[0] == 2 /* _NET_WM_STATE_TOGGLE */ &&
                              (!c->is_fullscreen || c->isfakefullscreen))));
}

/* Helper: Handle _NET_ACTIVE_WINDOW message for regular windows */
static void handle_active_window_regular(Client *c) {
    unsigned int i;

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

/* Helper: Handle _NET_ACTIVE_WINDOW message */
static void handle_active_window(Client *c) {
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
        handle_active_window_regular(c);
    }
}

/* Handle X client messages (systray, fullscreen, window activation). */
void clientmessage(XEvent *e) {
    XClientMessageEvent *cme = &e->xclient;
    Client *c = wintoclient(cme->window);

    /* Handle systray dock request */
    if (showsystray && cme->window == systray->win &&
        cme->message_type == netatom[NetSystemTrayOP]) {
        if (cme->data.l[1] == SYSTEM_TRAY_REQUEST_DOCK)
            handle_systray_dock_request(cme);
        return;
    }

    if (!c)
        return;

    if (cme->message_type == netatom[NetWMState])
        handle_netWMstate(c, cme);
    else if (cme->message_type == netatom[NetActiveWindow])
        handle_active_window(c);
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
                    else if (c->is_fullscreen)
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
            c->border_width = ev->border_width;
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

/** Reset bar state when mouse moves away from top area. */
static void handle_bar_leave_reset(XCrossingEvent *ev, int *barleavestatus) {
    if (*barleavestatus && ev->y_root >= selmon->my + 5) {
        resetbar();
        *barleavestatus = 0;
    }
}

/** Handle focus-follows-mouse for floating windows.
 *  Returns the client to focus, or NULL if focus should not change. */
static Client *handle_floating_focus(XCrossingEvent *ev, Client *c) {
    int resizeexit;

    if (!(c && selmon->sel &&
          (selmon->sel->isfloating || !tiling_layout_func(selmon)) &&
          c != selmon->sel &&
          (ev->window == root || visible(c) || ISVISIBLE(c) ||
           selmon->sel->issticky)))
        return c;

    resizeexit = hoverresizemouse(NULL);
    if (focusfollowsfloatmouse) {
        if (resizeexit)
            return NULL; /* Resize was performed, don't change focus */
        Client *newc = getcursorclient();
        if (newc && newc != selmon->sel)
            return newc;
    } else {
        return NULL; /* Don't change focus for floating windows */
    }
    return c;
}

/** Handle switching focus between monitors. Returns 1 if monitor changed. */
static int enternotify_monitor_switch(XCrossingEvent *ev, Client *c) {
    Monitor *m = c ? c->mon : wintomon(ev->window);
    if (m != selmon) {
        unfocus(selmon->sel, 1);
        selmon = m;
        return 1;
    }
    return 0;
}

/** Check if focus change should be skipped for floating windows.
 *  Returns 1 if focus should be skipped. */
static int should_skip_floating_focus(XCrossingEvent *ev, Client *c) {
    if (!focusfollowsfloatmouse) {
        if (ev->window != root && selmon->sel && c && c->isfloating &&
            selmon->lt[selmon->sellt] != (Layout *)&layouts[6])
            return 1;
    }
    return 0;
}

/**
 * Handle mouse enter events for focus-follows-mouse behavior.
 * This is triggered when the mouse cursor enters a window.
 */
void enternotify(XEvent *e) {
    Client *c;
    XCrossingEvent *ev = &e->xcrossing;
    static int barleavestatus = 0;

    /* Reset bar state when mouse leaves top area */
    handle_bar_leave_reset(ev, &barleavestatus);

    /* Filter invalid crossing events */
    if ((ev->mode != NotifyNormal || ev->detail == NotifyInferior) &&
        ev->window != root)
        return;

    c = wintoclient(ev->window);

    /* Handle floating window resize hover and focus */
    c = handle_floating_focus(ev, c);
    if (c == NULL)
        return;

    /* Focus follows mouse must be enabled */
    if (!focusfollowsmouse)
        return;

    /* Handle monitor switching */
    if (enternotify_monitor_switch(ev, c)) {
        focus(NULL);
        return;
    }

    /* Skip focus change for floating windows if configured */
    if (should_skip_floating_focus(ev, c))
        return;

    /* Skip if no client or same client */
    if (!c || c == selmon->sel)
        return;

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
        double titlewidth =
            (1.0 / (double)selmon->bt) * selmon->bar_clients_width;
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
                x += (1.0 / (double)selmon->bt) * selmon->bar_clients_width;
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
    } else if (selmon->sel && ev->x_root < selmon->mx + 60 + tagwidth +
                                               selmon->bar_clients_width) {
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

static void handle_focus_monitor(XButtonPressedEvent *ev) {
    Monitor *m;
    if ((m = wintomon(ev->window)) && m != selmon) {
        /* if focus doesn't follow the mouse, the scroll wheel shouldn't switch
         * focus */
        if (focusfollowsmouse || ev->button <= Button3) {
            unfocus(selmon->sel, 1);
            selmon = m;
            focus(NULL);
        }
    }
}

static void handle_bar_click(XButtonPressedEvent *ev, unsigned int *click,
                             Arg *arg) {
    unsigned int i, x, occupied_tags = 0;
    Client *c;
    Monitor *m = selmon; /* Since ev->window == selmon->barwin, m is selmon */
    int blw = get_blw(selmon);

    i = 0;
    x = startmenusize;
    for (c = m->clients; c; c = c->next)
        occupied_tags |= c->tags == 255 ? 0 : c->tags;
    do {
        /* do not reserve space for vacant tags */
        if (i >= 9)
            continue;
        if (selmon->showtags) {
            if (!(occupied_tags & 1 << i || m->tagset[m->seltags] & 1 << i))
                continue;
        }

        x += TEXTW(tags[i]);
    } while (ev->x >= x && ++i < numtags);
    if (ev->x < startmenusize) {
        *click = ClkStartMenu;
        selmon->gesture = GestureNone;
        drawbar(selmon);
    } else if (i < numtags) {
        *click = ClkTagBar;
        arg->ui = 1 << i;
    } else if (ev->x < x + blw)
        *click = ClkLtSymbol;
    else if (!selmon->sel && ev->x > x + blw && ev->x < x + blw + bh)
        *click = ClkShutDown;
    /* 2px right padding */
    else if (ev->x > selmon->ww - getsystraywidth() - statuswidth + lrpad - 2)
        *click = ClkStatusText;
    else {
        if (selmon->stack) {
            x += blw;
            c = m->clients;

            do {
                if (!ISVISIBLE(c))
                    continue;
                else
                    x += (1.0 / (double)m->bt) * m->bar_clients_width;
            } while (ev->x > x && (c = c->next));

            if (c) {
                arg->v = c;
                double titlewidth =
                    (1.0 / (double)m->bt) * m->bar_clients_width;
                int title_start = x - titlewidth;
                int resize_start = title_start + titlewidth - 30;

                if (c != selmon->sel || ev->x < title_start + 32) {
                    *click = ClkCloseButton;
                } else if (ev->x > resize_start) {
                    *click = ClkResizeWidget;
                } else {
                    *click = ClkWinTitle;
                }
            }
        } else {
            *click = ClkRootWin;
        }
    }
}

static void handle_client_click(XButtonPressedEvent *ev, Client *c,
                                unsigned int *click) {
    if (focusfollowsmouse || ev->button <= Button3) {
        focus(c);
        restack(selmon);
    }
    XAllowEvents(dpy, ReplayPointer, CurrentTime);
    *click = ClkClientWin;
}

void buttonpress(XEvent *e) {
    unsigned int i, click;
    Arg arg = {0};
    Client *c;
    XButtonPressedEvent *ev = &e->xbutton;

    click = ClkRootWin;
    /* focus monitor if necessary */
    handle_focus_monitor(ev);

    if (ev->window == selmon->barwin) {
        handle_bar_click(ev, &click, &arg);
    } else if ((c = wintoclient(ev->window))) {
        handle_client_click(ev, c, &click);
    } else if (ev->x > selmon->mx + selmon->mw - 50) {
        click = ClkSideBar;
    }
    // Handle resize click when cursor is in resize mode near floating window
    if (click == ClkRootWin && altcursor == AltCurResize &&
        ev->button == Button1 && selmon->sel &&
        (selmon->sel->isfloating || !tiling_layout_func(selmon))) {
        resetcursor();
        resizemouse(NULL);
        return;
    }
    /* Execute the button action handler that matches the click location,
     * button, and modifier keys. For special click types (tags, window titles,
     * buttons), pass the constructed arg with contextual data; otherwise use
     * button's arg.
     */
    for (i = 0; i < buttons_len; i++)
        if (click == buttons[i].click && buttons[i].func &&
            buttons[i].button == ev->button &&
            CLEANMASK(buttons[i].mask) == CLEANMASK(ev->state))
            buttons[i].func((click == ClkTagBar || click == ClkWinTitle ||
                             click == ClkCloseButton || click == ClkShutDown ||
                             click == ClkSideBar || click == ClkResizeWidget) &&
                                    buttons[i].arg.i == 0
                                ? &arg
                                : &buttons[i].arg);
}

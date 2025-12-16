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

#include "animation.h"
#include "bar.h"
#include "floating.h"
#include "focus.h"
#include "instantwm.h"
#include "layouts.h"
#include "mouse.h"
#include "overlay.h"
#include "scratchpad.h"
#include "systray.h"
#include "tags.h"
#include "toggles.h"
#include "util.h"
#include "xresources.h"

/* configuration, allows nested code to access above variables */
#include "config.h"

/* variables */
unsigned int tagmask = TAGMASK;
int numtags = LENGTH(tags);

Systray *systray = NULL;
static const char broken[] = "broken";
char stext[1024];

int showalttag = 0;
int freealttab = 0;

Client *lastclient;

int tagprefix = 0;
int bardragging = 0;
int altcursor = 0;
int tagwidth = 0;
int doubledraw = 0;
static int desktopicons = 0;
static int newdesktop = 0;
int pausedraw = 0;

int statuswidth = 0;

static int isdesktop = 0;

static int screen;
static int sw, sh; /* X display screen geometry width, height */
int bh;            /* bar height */
int lrpad;         /* sum of left and right padding for text */
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
Atom wmatom[WMLast], netatom[NetLast], xatom[XLast], motifatom;
static int running = 1;
Cur *cursor[CurLast];
Clr ***tagscheme;
Clr ***windowscheme;
Clr ***closebuttonscheme;
Clr *borderscheme; /* exported for modules */
Clr *statusscheme;

Display *dpy;
Drw *drw;
Monitor *mons; /* exported for modules */
Window root;   /* exported for modules */
static Window wmcheckwin;
int focusfollowsmouse = 1; /* exported for modules */
int focusfollowsfloatmouse = 1;
static int barleavestatus = 0;
int animated = 1;
int specialnext = 0;

Client *animclient;

int commandoffsets[20];

int forceresize = 0;
Monitor *selmon;

/* Pertag is now defined in instantwm.h */

/* compile-time check if all tags fit into an unsigned int bit array. */
struct NumTags {
    char limitexceeded[LENGTH(tags) > 31 ? -1 : 1];
};

void keyrelease(XEvent *e) {}

/* overlayexists() moved to overlay.c */

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

/* resetsnap() moved to floating.c */
/* saveallfloating() moved to floating.c */
/* restoreallfloating() moved to floating.c */
/* applysnap() moved to floating.c */
/* checkfloating() moved to floating.c */
/* visible() moved to floating.c */
/* changesnap() moved to floating.c */
/* tempfullscreen() moved to floating.c */

int get_blw(Monitor *m) { return TEXTW(m->ltsymbol) * 1.5; }

/* directionfocus() moved to focus.c */

/* createoverlay() moved to overlay.c */
/* resetoverlay() moved to overlay.c */
/* easeOutCubic() moved to animation.c */
/* checkanimate() moved to animation.c */
/* animateclient() moved to animation.c */
/* showoverlay() moved to overlay.c */
/* hideoverlay() moved to overlay.c */
/* setoverlay() moved to overlay.c */

/* focuslastclient() moved to focus.c */

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
                    centerwindow(NULL);
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
                    selmon->activescratchpad = c;
                    centerwindow(NULL);
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
    if (altcursor == AltCurNone)
        return;
    XDefineCursor(dpy, root, cursor[CurNormal]->cursor);
    altcursor = AltCurNone;
}

void buttonpress(XEvent *e) {
    unsigned int i, x, click, occ = 0;
    Arg arg = {0};
    Client *c;
    Monitor *m;
    XButtonPressedEvent *ev = &e->xbutton;
    int blw = get_blw(selmon);

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
    // Handle resize click when cursor is in resize mode near floating window
    if (click == ClkRootWin && altcursor == AltCurResize &&
        ev->button == Button1 && selmon->sel &&
        (selmon->sel->isfloating || !selmon->lt[selmon->sellt]->arrange)) {
        resetcursor();
        resizemouse(NULL);
        return;
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

    // TODO figure out how to do this with the custom theming code (this only
    // frees dwm schemes)
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
            showoverlay(NULL);
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

    for (i = 0; i < MAX_TAGS; i++) {
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

/* clickstatus() moved to bar.c */

/* drawstatusbar() moved to bar.c */

/* drawbar() moved to bar.c */

/* drawbars() moved to bar.c */

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

/* getsystraywidth() moved to systray.c */

int gettextprop(Window w, Atom atom, char *text, unsigned int size) {
    char **list = NULL;
    int n;
    XTextProperty name;

    if (!text || size == 0)
        return 0;
    text[0] = '\0';
    if (!XGetTextProperty(dpy, w, &name, atom) || !name.nitems)
        return 0;
    if (name.encoding == XA_STRING) {
        strncpy(text, (char *)name.value, size - 1);
    } else if (XmbTextPropertyToTextList(dpy, &name, &list, &n) >= Success &&
               n > 0 && *list) {
        strncpy(text, *list, size - 1);
        XFreeStringList(list);
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
        unsigned int i, j, k;
        unsigned int modifiers[] = {0, LockMask, numlockmask,
                                    numlockmask | LockMask};
        int start, end, skip;
        KeySym *syms;

        XUngrabKey(dpy, AnyKey, AnyModifier, root);
        XDisplayKeycodes(dpy, &start, &end);
        syms = XGetKeyboardMapping(dpy, start, end - start + 1, &skip);
        if (!syms)
            return;

        for (k = start; k <= end; k++) {
            for (i = 0; i < LENGTH(keys); i++) {
                if (keys[i].keysym == syms[(k - start) * skip])
                    for (j = 0; j < LENGTH(modifiers); j++) {
                        if (freealttab && keys[i].mod == Mod1Mask)
                            continue;
                        XGrabKey(dpy, k, keys[i].mod | modifiers[j], root, True,
                                 GrabModeAsync, GrabModeAsync);
                    }
            }

            // add keyboard shortcuts without modifiers when tag is empty
            if (!selmon->sel) {
                for (i = 0; i < LENGTH(dkeys); i++) {
                    if (dkeys[i].keysym == syms[(k - start) * skip])
                        for (j = 0; j < LENGTH(modifiers); j++)
                            XGrabKey(dpy, k, dkeys[i].mod | modifiers[j], root,
                                     True, GrabModeAsync, GrabModeAsync);
                }
            }
        }

        XFree(syms);
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

    if (c->x + WIDTH(c) > c->mon->wx + c->mon->ww)
        c->x = c->mon->wx + c->mon->ww - WIDTH(c);
    if (c->y + HEIGHT(c) > c->mon->wy + c->mon->wh)
        c->y = c->mon->wy + c->mon->wh - HEIGHT(c);
    c->x = MAX(c->x, c->mon->wx);
    /* only fix client y-offset, if the client center might cover the bar */
    c->y = MAX(c->y, c->mon->wy);
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

    if (!XGetWindowAttributes(dpy, ev->window, &wa) || wa.override_redirect)
        return;

    if (!wintoclient(ev->window))
        manage(ev->window, &wa);
}

// Helper: handle hover near floating window for resize cursor
// Returns 1 if handled (should return from motionnotify), 0 otherwise
static int handlefloatingresizehover(Monitor *m) {
    if (!(selmon->sel && (selmon->sel->isfloating ||
                          NULL == selmon->lt[selmon->sellt]->arrange)))
        return 0;

    Client *c;
    int tilefound = 0;
    for (c = m->clients; c; c = c->next) {
        if (ISVISIBLE(c) &&
            !(c->isfloating || NULL == selmon->lt[selmon->sellt]->arrange)) {
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
        return 1; // Handled
    } else if (altcursor == AltCurResize) {
        resetcursor();
    }
    return 0;
}

// Helper: handle sidebar slider cursor
// Returns 1 if handled (should return from motionnotify), 0 otherwise
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

// Helper: handle overlay corner gesture
// Returns 1 if handled (should return from motionnotify), 0 otherwise
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
        selmon->gesture = 0;
        return 1;
    }
    return 0;
}

// Helper: handle tag bar hover (start menu and tag indicators)
static void handletagbarhover(XMotionEvent *ev) {
    if (selmon->hoverclient)
        selmon->hoverclient = NULL;

    if (ev->x_root < selmon->mx + tagwidth && !selmon->showtags) {
        // hover over start menu
        if (ev->x_root < selmon->mx + startmenusize) {
            selmon->gesture = 13;
            drawbar(selmon);
        } else {
            // hover over tag indicators
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

// Helper: handle window title bar hover (close button, resize widget, client
// hover)
static void handletitlebarhover(XMotionEvent *ev) {
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
        // hover over resize widget on title bar
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

    // indicator when hovering over clients
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
            selmon->gesture = 0;
            drawbar(selmon);
        }
    }
}

// gets triggered on mouse movement
void motionnotify(XEvent *e) {
    Monitor *m;
    XMotionEvent *ev = &e->xmotion;

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

    // hover below bar (in desktop area)
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
        barleavestatus = 1;
    }

    // hover in bar area
    if (handleoverlaygesture(ev))
        return;

    // cursor is to the left of window titles (tags area)
    if (ev->x_root < selmon->mx + tagwidth + 60) {
        handletagbarhover(ev);
    } else if (selmon->sel &&
               ev->x_root < selmon->mx + 60 + tagwidth + selmon->btw) {
        // cursor is on window titles
        handletitlebarhover(ev);
    } else {
        resetbar();
    }
}

/* resetbar() moved to bar.c */

// drag a window around using the mouse
/* movemouse() moved to mouse.c */

// drag up and down on the desktop to
// change volume or start onboard by dragging to the right
/* gesturemouse() moved to mouse.c */

// hover over the border to move/resize a window
/* resizeborder() moved to mouse.c */

// drag clients around the top bar
/* dragmouse() moved to mouse.c */

/* resetoverlaysize() moved to overlay.c */

// drag on the top bar with the right mouse
/* dragrightmouse() moved to mouse.c */

// drag out an area using slop and resize the selected window to it.
/* drawwindow() moved to mouse.c */

// drag the green tag mark to another tag
/* dragtag() moved to mouse.c */

void shutkill(const Arg *arg) {
    if (!selmon->clients)
        spawn(&((Arg){.v = instantshutdowncmd}));
    else
        killclient(arg);
}

/* nametag() moved to tags.c */

/* resetnametag() moved to tags.c */

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

/* removesystrayicon() moved to systray.c */

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

/* forceresizemouse() moved to mouse.c */

/* resizemouse() moved to mouse.c */

/* resizeaspectmouse() moved to mouse.c */

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

/* gettagwidth() moved to tags.c */

/* getxtag() moved to tags.c */

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

                strcpy(
                    tmpstring,
                    tagcolors[schemehovertypes[i].type][schemetagtypes[u].type]
                             [schemecolortypes[q].type]);

                tagcolors[schemehovertypes[i].type][schemetagtypes[u].type]
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

    struct sigaction sa;

    /* do not transform children into zombies when they terminate */
    sigemptyset(&sa.sa_mask);
    sa.sa_flags = SA_NOCLDSTOP | SA_NOCLDWAIT | SA_RESTART;
    sa.sa_handler = SIG_IGN;
    sigaction(SIGCHLD, &sa, NULL);

    /* clean up any zombies (inherited from .xinitrc etc) immediately */
    while (waitpid(-1, NULL, WNOHANG) > 0)
        ;

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

void spawn(const Arg *arg) {
    struct sigaction sa;
    if (arg->v == instantmenucmd)
        instantmenumon[0] = '0' + selmon->num;
    if (fork() == 0) {
        if (dpy)
            close(ConnectionNumber(dpy));
        setsid();
        sigemptyset(&sa.sa_mask);
        sa.sa_flags = 0;
        sa.sa_handler = SIG_DFL;
        sigaction(SIGCHLD, &sa, NULL);
        execvp(((char **)arg->v)[0], (char **)arg->v);
        die("instantwm: execvp '%s' failed:", ((char **)arg->v)[0]);
    }
}

/* computeprefix() moved to tags.c */

void setclienttagprop(Client *c) {
    long data[] = {(long)c->tags, (long)c->mon->num};
    XChangeProperty(dpy, c->win, netatom[NetClientInfo], XA_CARDINAL, 32,
                    PropModeReplace, (unsigned char *)data, 2);
}

/* tag() moved to tags.c */

/* tagall() moved to tags.c */

/* followtag() moved to tags.c */

/* swaptags() moved to tags.c */

/* followview() moved to tags.c */

/* resetsticky() moved to tags.c */

/* tagmon() moved to tags.c */

/* setoverlaymode() moved to overlay.c */

/* tagtoleft() moved to tags.c */

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

/* tagtoright() moved to tags.c */

/* ctrltoggle() moved to toggles.c */

void setspecialnext(const Arg *arg) { specialnext = arg->ui; }

/* togglealttag() moved to toggles.c */
/* alttabfree() moved to toggles.c */
/* togglesticky() moved to toggles.c */
/* toggleprefix() moved to toggles.c */
/* toggleanimated() moved to toggles.c */
/* setborderwidth() moved to toggles.c */
/* togglefocusfollowsmouse() moved to toggles.c */
/* togglefocusfollowsfloatmouse() moved to toggles.c */
/* toggledoubledraw() moved to toggles.c */
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
/* togglelocked() moved to toggles.c */

/* warp() moved to focus.c */
/* forcewarp() moved to focus.c */
/* warpinto() moved to focus.c */
/* warpfocus() moved to focus.c */

/* moveresize() moved to floating.c */

/* keyresize() moved to floating.c */

/* centerwindow() moved to floating.c */

/* toggleshowtags() moved to toggles.c */

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
        showoverlay(NULL);
        animated = tmpnoanim;
    }
}

/* savefloating() moved to floating.c */
/* restorefloating() moved to floating.c */
/* savebw() moved to floating.c */
/* restorebw() moved to floating.c */
/* applysize() moved to floating.c */
/* togglefloating() moved to floating.c */
/* changefloating() moved to floating.c */

/* toggletag() moved to tags.c */

/* updatescratchvisible() moved to scratchpad.c */
/* findnamedscratchpad() moved to scratchpad.c */
/* makescratchpad() moved to scratchpad.c */
/* togglescratchpad() moved to scratchpad.c */
/* createscratchpad() moved to scratchpad.c */
/* showscratchpad() moved to scratchpad.c */
/* hidescratchpad() moved to scratchpad.c */
/* scratchpadstatus() moved to scratchpad.c */

/* updatestatus() moved to bar.c */

/* toggleview() moved to tags.c */

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

    if (c == m->activescratchpad)
        m->activescratchpad = NULL;

    detach(c);
    detachstack(c);
    if (!destroyed) {
        wc.border_width = c->oldbw;
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

/* updatebarpos() moved to bar.c */

void updateclientlist(void) {
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

/* updatesystrayicongeom() moved to systray.c */

/* updatesystrayiconstate() moved to systray.c */

/* updatesystray() moved to systray.c */

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

/* view() moved to tags.c */

/* moveleft() moved to tags.c */

/* viewtoleft() moved to tags.c */

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

/* shiftview() moved to tags.c */

/* viewtoright() moved to tags.c */

/* moveright() moved to tags.c */

/* upscaleclient() moved to floating.c */

/* downscaleclient() moved to floating.c */

/* scaleclient() moved to floating.c */

// toggle overview like layout
/* overtoggle() moved to tags.c */

/* lastview() moved to tags.c */

/* fullovertoggle() moved to tags.c */

Client *wintoclient(Window w) {
    Client *c;
    Monitor *m;

    for (m = mons; m; m = m->next)
        for (c = m->clients; c; c = c->next)
            if (c->win == w)
                return c;
    return NULL;
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

/* winview() moved to tags.c */

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

/* systraytomon() moved to systray.c */

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

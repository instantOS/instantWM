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
#include "client.h"
#include "events.h"
#include "floating.h"
#include "focus.h"
#include "instantwm.h"
#include "keyboard.h"
#include "layouts.h"
#include "monitors.h"
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
size_t keys_len = LENGTH(keys);
size_t dkeys_len = LENGTH(dkeys);

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

int pausedraw = 0;

int statuswidth = 0;

static int isdesktop = 0;

static int screen;
int sw, sh; /* X display screen geometry width, height */
int bh;     /* bar height */
int lrpad;  /* sum of left and right padding for text */
static int (*xerrorxlib)(Display *, XErrorEvent *);
unsigned int numlockmask = 0;
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

int get_blw(Monitor *m) { return TEXTW(m->ltsymbol) * 1.5; }

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
        case SpecialFloat:
            c->isfloating = 1;
            break;
        }
        specialnext = SpecialNone;
    } else {
        unsigned int i;
        const Rule *r;
        for (i = 0; i < LENGTH(rules); i++) {
            r = &rules[i];
            if ((!r->title || strstr(c->name, r->title)) &&
                (!r->class || strstr(class, r->class)) &&
                (!r->instance || strstr(instance, r->instance))) {
                if (strstr(r->class, "Onboard") != NULL) {
                    c->issticky = 1;
                }

                switch (r->isfloating) {
                case RuleFloatCenter:
                    selmon->sel = c;
                    c->isfloating = 1;
                    centerwindow(NULL);
                    break;
                case RuleFloatFullscreen:
                    /* fullscreen overlay */
                    selmon->sel = c;
                    c->isfloating = 1;
                    c->w = c->mon->mw;
                    c->h = c->mon->wh - (selmon->showbar ? bh : 0);
                    if (selmon->showbar)
                        c->y = selmon->my + bh;
                    c->x = selmon->mx;
                    break;
                case RuleScratchpad:
                    selmon->sel = c;
                    c->tags = SCRATCHPAD_MASK;
                    selmon->scratchvisible = 1;
                    c->issticky = 1;
                    c->isfloating = 1;
                    selmon->activescratchpad = c;
                    centerwindow(NULL);
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

void resetcursor() {
    if (altcursor == AltCurNone)
        return;
    XDefineCursor(dpy, root, cursor[CurNormal]->cursor);
    altcursor = AltCurNone;
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
    unsigned int i, x, occ = 0;
    Client *c;
    Monitor *m = selmon; /* Since ev->window == selmon->barwin, m is selmon */
    int blw = get_blw(selmon);

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
        *click = ClkStartMenu;
        selmon->gesture = GestureNone;
        drawbar(selmon);
    } else if (i < LENGTH(tags)) {
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
                    x += (1.0 / (double)m->bt) * m->btw;
            } while (ev->x > x && (c = c->next));

            if (c) {
                arg->v = c;
                if (c != selmon->sel ||
                    ev->x > x - (1.0 / (double)m->bt) * m->btw + 32) {
                    *click = ClkWinTitle;
                } else {
                    *click = ClkCloseButton;
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
        (selmon->sel->isfloating || !selmon->lt[selmon->sellt]->arrange)) {
        resetcursor();
        resizemouse(NULL);
        return;
    }
    // TODO: document what this does and why it does it
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

void distributeclients(const Arg *arg) {
    Client *c;
    int tagcounter = 0;
    focus(NULL);

    for (c = selmon->clients; c; c = c->next) {
        // overlays or scratchpads aren't on regular tags anyway
        if (c == selmon->overlay || c->tags & SCRATCHPAD_MASK)
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
        if (c->tags & SCRATCHPAD_MASK) {
            selmon->activescratchpad = c;
        }
    } else {
        XSetInputFocus(dpy, root, RevertToPointerRoot, CurrentTime);
        XDeleteProperty(dpy, root, netatom[NetActiveWindow]);
    }
    selmon->sel = c;
    if (selmon->gesture != GestureOverlay && selmon->gesture)
        selmon->gesture = GestureNone;

    if (selmon->gesture < GestureOverlay)
        selmon->gesture = GestureNone;
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
            case CmdArgNone:
                arg = commands[i].arg;
                break;
            case CmdArgToggle:
                argnum = atoi(fcursor);
                if (argnum != 0 && fcursor[0] != '0') {
                    arg = ((Arg){.ui = atoi(fcursor)});
                } else {
                    arg = commands[i].arg;
                }
                break;
            case CmdArgTag:
                argnum = atoi(fcursor);
                if (argnum != 0 && fcursor[0] != '0') {
                    arg = ((Arg){.ui = (1 << (atoi(fcursor) - 1))});
                } else {
                    arg = commands[i].arg;
                }
                break;
            case CmdArgString:
                arg = ((Arg){.v = fcursor});
                break;
            case CmdArgInt:
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

    c->saved_float_x = c->x;
    c->saved_float_y = c->y = c->y >= c->mon->my ? c->y : c->y + c->mon->my;
    c->saved_float_width = c->w;
    c->saved_float_height = c->h;
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

void shutkill(const Arg *arg) {
    if (!selmon->clients)
        spawn(&((Arg){.v = instantshutdowncmd}));
    else
        killclient(arg);
}

void quit(const Arg *arg) { running = 0; }

void resizebarwin(Monitor *m) {
    unsigned int w = m->ww;
    if (showsystray && m == systraytomon(m))
        w -= getsystraywidth();
    XMoveResizeWindow(dpy, m->barwin, m->wx, m->by, w, bh);
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

void setspecialnext(const Arg *arg) { specialnext = arg->ui; }

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

// TODO: can this be moved?
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

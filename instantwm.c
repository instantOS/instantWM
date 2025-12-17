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
#include "globals.h"
#include "keyboard.h"
#include "layouts.h"
#include "monitors.h"
#include "mouse.h"
#include "overlay.h"
#include "push.h"
#include "scratchpad.h"
#include "systray.h"
#include "tags.h"
#include "toggles.h"
#include "util.h"
#include "xresources.h"

/* variables */
/* defined in config.c */

Systray *systray = NULL;
const char broken[] = "broken";
char stext[1024];

int showalttag = 0;
int freealttab = 0;

Client *lastclient;

int tagprefix = 0;
int bar_dragging = 0;
int altcursor = 0;
int tagwidth = 0;
int doubledraw = 0;

int pausedraw = 0;

int statuswidth = 0;

static int isdesktop = 0;

int screen;
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
                                        [UnmapNotify] = unmapnotify,
                                        [LeaveNotify] = leavenotify};
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

int commandoffsets[20];

int force_resize = 0;
Monitor *selmon;

void resetcursor() {
    if (altcursor == AltCurNone)
        return;
    XDefineCursor(dpy, root, cursor[CurNormal]->cursor);
    altcursor = AltCurNone;
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

    /* tagcolors size is fixed 2x5x3 */
    for (i = 0; i < 2; i++) {
        for (u = 0; u < 5; u++) {
            free(tagscheme[i][u]);
        }
    }

    /* windowcolors size is fixed 2x7x3 */
    for (i = 0; i < 2; i++) {
        for (u = 0; u < 7; u++) {
            free(windowscheme[i][u]);
        }
    }

    /* closebuttoncolors size is fixed 2x3x3 */
    for (i = 0; i < 2; i++) {
        for (u = 0; u < 3; u++) {
            free(closebuttonscheme[i][u]);
        }
    }

    free(statusscheme);
    free(borderscheme);

    /* Legacy dwm color scheme cleanup is disabled because instantWM uses custom
     * theming (windowscheme, closebuttonscheme) which are already freed above.
     * Re-enabling this would cause double-free errors.
     */
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
        if (c && 1 << tagcounter & tagmask) {
            c->tags = 1 << tagcounter & tagmask;
        }
        tagcounter++;
    }
    focus(NULL);
    arrange(selmon);
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

/* Cycle focus through visible clients on the current monitor.
 * arg->i > 0: Focus next visible client (wrap to first if at end)
 * arg->i <= 0: Focus previous visible client (wrap to last if at start)
 * Does nothing if no client is selected or if fullscreen (non-fake). */
void focusstack(const Arg *arg) {
    Client *c = NULL, *i;

    if (!selmon->sel ||
        (selmon->sel->is_fullscreen && !selmon->sel->isfakefullscreen))
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
        for (i = 0; i < buttons_len; i++)
            if (buttons[i].click == ClkClientWin)
                for (j = 0; j < LENGTH(modifiers); j++)
                    XGrabButton(dpy, buttons[i].button,
                                buttons[i].mask | modifiers[j], c->win, False,
                                BUTTONMASK, GrabModeAsync, GrabModeSync, None,
                                None);
    }
}

void quit(const Arg *arg) { running = 0; }

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
    if (!drw_fontset_create(drw, fonts, fonts_len))
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

/* See LICENSE file for copyright and license details. */
#define _POSIX_C_SOURCE 200809L

#include <X11/XF86keysym.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "floating.h"
#include "instantwm.h"
#include "layouts.h"
#include "mouse.h"
#include "util.h"

/* extern variables */
extern Monitor *selmon;
extern Monitor *mons;
extern Display *dpy;
extern Window root;
extern int bh;
extern int lrpad;
extern int doubledraw;
extern int animated;
extern const int showsystray;
extern const unsigned int systraypinning;
extern const unsigned int systrayspacing;
extern int forceresize;
extern Cur *cursor[CurLast];
extern void (*handler[LASTEvent])(XEvent *);
extern const unsigned int snap;
extern const int resizehints;
extern const char *upvol[];
extern const char *downvol[];
extern int tagwidth;
extern unsigned int tagmask;
extern int bardragging;

/* function declarations */
extern void tempfullscreen(const Arg *arg);
extern void resetsnap(Client *c);
extern void resize(Client *c, int x, int y, int w, int h, int interact);
extern void restack(Monitor *m);
extern int getrootptr(int *x, int *y);
extern void spawn(const Arg *arg);
extern void drawbar(Monitor *m);
extern void updatebarpos(Monitor *m);
extern void tag(const Arg *arg);
extern void view(const Arg *arg);
extern void tagall(const Arg *arg);
extern void followtag(const Arg *arg);
extern int getxtag(int ix);
extern int gettagwidth();
extern void createoverlay();
extern void setoverlay(Client *c);
extern Monitor *recttomon(int x, int y, int w, int h);
extern void sendmon(Client *c, Monitor *m);
extern void resizeclient(Client *c, int x, int y, int w, int h);
extern void savefloating(Client *c);
extern void togglefloating(const Arg *arg);
extern void unfocus(Client *c, int setfocus);
extern void focus(Client *c);
extern void resetbar();

/* Drag loop types and generic implementation */
typedef struct {
    int extra_mask; /* Extra mask bits (e.g., KeyPressMask) */
    void *data;     /* Opaque pointer for callback use */
} DragContext;

typedef enum {
    DRAG_CONTINUE = 0, /* Continue the loop */
    DRAG_BREAK = 1,    /* Exit the loop (custom condition) */
} DragResult;

typedef DragResult (*DragMotionHandler)(XEvent *ev, void *data);
typedef DragResult (*DragExtraHandler)(XEvent *ev, void *data);

/**
 * Generic drag loop that handles the common event dispatch pattern.
 * All loops use unified rate limiting: doubledraw ? REFRESH_RATE_HI :
 * REFRESH_RATE_LO
 *
 * @param ctx           Context containing extra mask and user data (can be
 * NULL)
 * @param motion_handler Callback for MotionNotify events (can be NULL)
 * @param extra_handler  Callback for events other than standard set (can be
 * NULL) Called for KeyPress, ButtonPress if in extra_mask
 * @return 1 if loop completed normally (ButtonRelease), 0 if broken early
 */
static int drag_loop(DragContext *ctx, DragMotionHandler motion_handler,
                     DragExtraHandler extra_handler) {
    XEvent ev;
    Time lasttime = 0;
    int mask = MOUSEMASK | ExposureMask | SubstructureRedirectMask;

    if (ctx && ctx->extra_mask)
        mask |= ctx->extra_mask;

    int rate = doubledraw ? REFRESH_RATE_HI : REFRESH_RATE_LO;

    do {
        XMaskEvent(dpy, mask, &ev);
        switch (ev.type) {
        case ConfigureRequest:
        case Expose:
        case MapRequest:
            handler[ev.type](&ev);
            break;
        case MotionNotify:
            if ((ev.xmotion.time - lasttime) <= (1000 / rate))
                continue;
            lasttime = ev.xmotion.time;
            if (motion_handler) {
                if (motion_handler(&ev, ctx ? ctx->data : NULL) == DRAG_BREAK)
                    return 0;
            }
            break;
        default:
            if (extra_handler) {
                if (extra_handler(&ev, ctx ? ctx->data : NULL) == DRAG_BREAK)
                    return 0;
            }
            break;
        }
    } while (ev.type != ButtonRelease);

    return 1;
}

/* Handle window drop on bar: move to tag or re-tile */
static void handle_bar_drop(Client *c) {
    int x, y;
    getrootptr(&x, &y);

    if (y < selmon->my || y >= selmon->my + bh)
        return;

    /* Check if dropped on a tag indicator */
    int droptag = getxtag(x);
    if (droptag >= 0 && x < selmon->mx + gettagwidth()) {
        /* Move window to that tag */
        tag(&((Arg){.ui = 1 << droptag}));

        /* Make floating windows tiled when dropped on tag indicator
         * Note: tag() calls focus(NULL) which changes selmon->sel,
         * so we must use settiled() which operates on the specific client
         */
        settiled(c, 1);
    } else if (c->isfloating) {
        /* Dropped elsewhere on bar - make it tiled again */
        togglefloating(NULL);
    }
}

/* Data structure for movemouse motion handler */
typedef struct {
    Client *c;
    int ocx, ocy;       /* Original client position */
    int startx, starty; /* Initial pointer position */
} MovemouseData;

static DragResult movemouse_motion(XEvent *ev, void *data) {
    MovemouseData *d = (MovemouseData *)data;
    Client *c = d->c;
    int nx, ny;

    nx = d->ocx + (ev->xmotion.x - d->startx);
    ny = d->ocy + (ev->xmotion.y - d->starty);

    /* If cursor is on the bar, offset window below the bar */
    if (ev->xmotion.y_root >= selmon->my &&
        ev->xmotion.y_root < selmon->my + bh) {
        ny = selmon->my + bh;
    }

    if (abs(selmon->wx - nx) < snap)
        nx = selmon->wx;
    else if (abs((selmon->wx + selmon->ww) - (nx + WIDTH(c))) < snap)
        nx = selmon->wx + selmon->ww - WIDTH(c);
    if (abs(selmon->wy - ny) < snap)
        ny = selmon->wy;
    else if (abs((selmon->wy + selmon->wh) - (ny + HEIGHT(c))) < snap)
        ny = selmon->wy + selmon->wh - HEIGHT(c);
    if (!c->isfloating && tiling_layout_func(selmon) &&
        (abs(nx - c->x) > snap || abs(ny - c->y) > snap)) {
        if (animated) {
            animated = 0;
            togglefloating(NULL);
            animated = 1;
        } else {
            togglefloating(NULL);
        }
    }
    if (!tiling_layout_func(selmon) || c->isfloating)
        resize(c, nx, ny, c->w, c->h, 1);

    return DRAG_CONTINUE;
}

void movemouse(const Arg *arg) {
    int x, y;
    Client *c;

    // some windows are immovable
    if (!(c = selmon->sel) || (c->isfullscreen && !c->isfakefullscreen) ||
        c == selmon->overlay)
        return;

    if (c == selmon->fullscreen) {
        tempfullscreen(NULL);
        return;
    }

    if (c->snapstatus) {
        resetsnap(c);
        return;
    }

    if (NULL == tiling_layout_func(selmon)) {
        // unmaximize in floating layout
        if (c->x >= selmon->mx - MAX_UNMAXIMIZE_OFFSET &&
            c->y >= selmon->my + bh - MAX_UNMAXIMIZE_OFFSET &&
            c->w >= selmon->mw - MAX_UNMAXIMIZE_OFFSET &&
            c->h >= selmon->mh - MAX_UNMAXIMIZE_OFFSET) {
            resize(c, c->saved_float_x, c->saved_float_y, c->saved_float_width,
                   c->saved_float_height, 0);
        }
    }

    restack(selmon);
    // make pointer grabby shape
    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurMove]->cursor, CurrentTime) != GrabSuccess)
        return;
    if (!getrootptr(&x, &y))
        return;

    MovemouseData data = {
        .c = c, .ocx = c->x, .ocy = c->y, .startx = x, .starty = y};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, movemouse_motion, NULL);
    XUngrabPointer(dpy, CurrentTime);

    /* Handle drop on bar (tag move, re-tile) */
    handle_bar_drop(c);

    handle_client_monitor_switch(c);
}

/* Data structure for gesturemouse motion handler */
typedef struct {
    int *lasty;
} GesturemouseData;

static DragResult gesturemouse_motion(XEvent *ev, void *data) {
    GesturemouseData *d = (GesturemouseData *)data;

    if (abs(*d->lasty - ev->xmotion.y_root) > selmon->mh / 30) {
        if (ev->xmotion.y_root < *d->lasty)
            spawn(&((Arg){.v = upvol}));
        else
            spawn(&((Arg){.v = downvol}));
        *d->lasty = ev->xmotion.y_root;
    }

    return DRAG_CONTINUE;
}

void gesturemouse(const Arg *arg) {
    int x, y, lasty;

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurMove]->cursor, CurrentTime) != GrabSuccess)
        return;
    if (!getrootptr(&x, &y))
        return;
    lasty = y;

    GesturemouseData data = {.lasty = &lasty};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, gesturemouse_motion, NULL);
    XUngrabPointer(dpy, CurrentTime);
}

// Check if cursor is in the resize border zone around the selected floating
// window Returns 1 if in border zone, 0 if not or no valid floating selection
int isinresizeborder() {
    if (!(selmon->sel &&
          (selmon->sel->isfloating || !tiling_layout_func(selmon))))
        return 0;
    int x, y;
    getrootptr(&x, &y);
    Client *c = selmon->sel;
    // Not in border if: on bar, inside window, or too far from window
    if ((selmon->showbar && y < selmon->my + bh) ||
        (y > c->y && y < c->y + c->h && x > c->x && x < c->x + c->w) ||
        y < c->y - RESIZE_BORDER_ZONE || x < c->x - RESIZE_BORDER_ZONE ||
        y > c->y + c->h + RESIZE_BORDER_ZONE ||
        x > c->x + c->w + RESIZE_BORDER_ZONE) {
        return 0;
    }
    return 1;
}

/* Data structure for hoverresizemouse handlers */
typedef struct {
    int inborder;
    int resize_started;
} HoverResizeData;

static DragResult hoverresize_motion(XEvent *ev, void *data) {
    HoverResizeData *d = (HoverResizeData *)data;

    if (!isinresizeborder()) {
        d->inborder = 0;
        Client *newc = getcursorclient();
        if (newc && newc != selmon->sel)
            focus(newc);
        return DRAG_BREAK;
    }

    return DRAG_CONTINUE;
}

static DragResult hoverresize_extra(XEvent *ev, void *data) {
    HoverResizeData *d = (HoverResizeData *)data;

    switch (ev->type) {
    case KeyPress:
        if (ev->xkey.keycode == KEYCODE_ESCAPE) {
            d->inborder = 0;
            return DRAG_BREAK;
        }
        handler[ev->type](ev);
        break;
    case ButtonPress:
        if (ev->xbutton.button == Button1) {
            XUngrabPointer(dpy, CurrentTime);
            resizemouse(NULL);
            d->resize_started = 1;
            return DRAG_BREAK;
        }
        break;
    }

    return DRAG_CONTINUE;
}

int hoverresizemouse(const Arg *arg) {
    if (!isinresizeborder())
        return 0;

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurResize]->cursor,
                     CurrentTime) != GrabSuccess)
        return 0;

    HoverResizeData data = {.inborder = 1, .resize_started = 0};
    DragContext ctx = {.extra_mask = KeyPressMask, .data = &data};

    drag_loop(&ctx, hoverresize_motion, hoverresize_extra);

    if (!data.resize_started)
        XUngrabPointer(dpy, CurrentTime);
    return 1;
}

static void drag_window_title(Client *c) {
    XUngrabPointer(dpy, CurrentTime);
    show(c);
    focus(c);
    restack(selmon);
    if (selmon->sel) {
        XWarpPointer(dpy, None, root, 0, 0, 0, 0,
                     selmon->sel->x + selmon->sel->w / 2,
                     selmon->sel->y + selmon->sel->h / 2);
        movemouse(NULL);
    }
}

static void click_window_title(Client *c, int was_hidden, int was_focused) {
    if (was_hidden) {
        show(c);
        focus(c);
        restack(selmon);
    } else {
        if (was_focused) {
            hide(c);
        } else {
            focus(c);
            restack(selmon);
        }
    }
}

/* Data structure for window_title_mouse_handler motion handler */
typedef struct {
    Client *c;
    int startx, starty;
    int was_hidden;
    int drag_started;
} WindowTitleData;

static DragResult window_title_motion(XEvent *ev, void *data) {
    WindowTitleData *d = (WindowTitleData *)data;
    int x, y;

    getrootptr(&x, &y);
    /* If mouse moved beyond threshold, start moving the window */
    if (abs(x - d->startx) > DRAG_THRESHOLD ||
        abs(y - d->starty) > DRAG_THRESHOLD) {
        if (d->was_hidden)
            show(d->c);
        drag_window_title(d->c);
        d->drag_started = 1;
        return DRAG_BREAK;
    }

    return DRAG_CONTINUE;
}

void window_title_mouse_handler(const Arg *arg) {
    int startx, starty;

    Client *c = (Client *)arg->v;
    if (!c)
        return;

    int was_focused = (selmon->sel == c);
    int was_hidden = HIDDEN(c);

    /* Grab pointer to detect if user drags or just clicks */
    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurNormal]->cursor,
                     CurrentTime) != GrabSuccess)
        return;
    if (!getrootptr(&startx, &starty)) {
        XUngrabPointer(dpy, CurrentTime);
        return;
    }

    WindowTitleData data = {.c = c,
                            .startx = startx,
                            .starty = starty,
                            .was_hidden = was_hidden,
                            .drag_started = 0};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, window_title_motion, NULL);

    /* If drag was started, drag_window_title already handled everything */
    if (data.drag_started)
        return;

    /* Button released without significant movement - just focus */
    XUngrabPointer(dpy, CurrentTime);
    click_window_title(c, was_hidden, was_focused);
}

/* Data structure for window_title_mouse_handler_right motion handler */
typedef struct {
    int startx, starty;
    int start_initialized;
    int dragging;
    int gesture_triggered;
} RightTitleData;

static DragResult right_title_motion(XEvent *ev, void *data) {
    RightTitleData *d = (RightTitleData *)data;
    XMotionEvent *motion = &ev->xmotion;

    if (!d->start_initialized) {
        d->startx = motion->x_root;
        d->starty = motion->y_root;
        d->start_initialized = 1;
    }

    if (abs(motion->x_root - d->startx) > selmon->mw / 20) {
        if (!d->dragging) {
            Arg a = {.i = (motion->x_root < d->startx) ? -1 : 1};
            tagtoleft(&a);
            d->dragging = 1;
        }
    } else if (abs(motion->y_root - d->starty) > GESTURE_THRESHOLD) {
        if (motion->y_root > d->starty) {
            hidewin(NULL);
        } else {
            killclient(NULL);
        }
        d->gesture_triggered = 1;
        return DRAG_BREAK;
    }

    return DRAG_CONTINUE;
}

void window_title_mouse_handler_right(const Arg *arg) {
    int x, y;

    Client *tempc = (Client *)arg->v;
    resetbar();
    if (tempc->isfullscreen &&
        !tempc->isfakefullscreen) /* no support moving fullscreen windows by
                                     mouse */
        return;

    focus(tempc);

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurMove]->cursor, CurrentTime) != GrabSuccess)
        return;
    if (!getrootptr(&x, &y))
        return;

    RightTitleData data = {.startx = 0,
                           .starty = 0,
                           .start_initialized = 0,
                           .dragging = 0,
                           .gesture_triggered = 0};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, right_title_motion, NULL);
    XUngrabPointer(dpy, CurrentTime);
}

/* Helper functions for drawwindow */
int parse_slop_output(const char *output, int dimensions[4]) {
    char strout[100] = {0};
    char tmpstring[30] = {0};
    int firstchar = 0;
    int counter = 0;
    int i;

    if (!output || strlen(output) < 6)
        return 0;

    strcpy(strout, output);

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
            memset(tmpstring, 0, sizeof(tmpstring));
        }
    }

    return counter == 4;
}

int is_valid_window_size(int x, int y, int width, int height, Client *c) {
    return (width > MIN_WINDOW_SIZE && height > MIN_WINDOW_SIZE &&
            x > -SLOP_MARGIN && y > -SLOP_MARGIN &&
            width < selmon->mw + SLOP_MARGIN &&
            height < selmon->mh + SLOP_MARGIN &&
            (abs(c->w - width) > 20 || abs(c->h - height) > 20 ||
             abs(c->x - x) > 20 || abs(c->y - y) > 20));
}

void handle_monitor_switch(Client *c, int x, int y, int width, int height) {
    Monitor *m;

    if ((m = recttomon(x, y, width, height)) != selmon) {
        sendmon(c, m);
        unfocus(selmon->sel, 0);
        selmon = m;
        focus(NULL);
    }
}

void handle_client_monitor_switch(Client *c) {
    Monitor *m;

    if ((m = recttomon(c->x, c->y, c->w, c->h)) != selmon) {
        sendmon(c, m);
        unfocus(selmon->sel, 0);
        selmon = m;
        focus(NULL);
    }
}

void apply_window_resize(Client *c, int x, int y, int width, int height) {
    if (c->isfloating) {
        resize(c, x, y, width, height, 1);
    } else {
        togglefloating(NULL);
        resize(c, x, y, width, height, 1);
    }
}

void drawwindow(const Arg *arg) {
    char str[100];
    char strout[100] = {0};
    int dimensions[4];
    int width, height, x, y;
    Client *c;
    FILE *fp;

    if (!selmon->sel)
        return;

    fp = popen("instantslop -f x%xx%yx%wx%hx", "r");
    if (!fp)
        return;

    while (fgets(str, 100, fp) != NULL) {
        strcat(strout, str);
    }
    pclose(fp);

    if (!parse_slop_output(strout, dimensions))
        return;

    x = dimensions[0];
    y = dimensions[1];
    width = dimensions[2];
    height = dimensions[3];

    if (!selmon->sel)
        return;

    c = selmon->sel;

    if (is_valid_window_size(x, y, width, height, c)) {
        handle_monitor_switch(c, x, y, width, height);
        apply_window_resize(c, x, y, width, height);
    }
}

/* Data structure for dragtag motion handler */
typedef struct {
    int *leftbar;
    int *tagx;
    XMotionEvent *last_motion; /* Store last motion for post-loop processing */
} DragtagData;

static DragResult dragtag_motion(XEvent *ev, void *data) {
    DragtagData *d = (DragtagData *)data;

    /* Store the motion event for post-loop use */
    d->last_motion = &ev->xmotion;

    if (ev->xmotion.y_root > selmon->my + bh + 1)
        *d->leftbar = 1;

    int newtag = getxtag(ev->xmotion.x_root);
    if (*d->tagx != newtag) {
        *d->tagx = newtag;
        selmon->gesture = newtag + 1;
        drawbar(selmon);
    }

    return *d->leftbar ? DRAG_BREAK : DRAG_CONTINUE;
}

void dragtag(const Arg *arg) {
    if (!tagwidth)
        tagwidth = gettagwidth();
    if ((arg->ui & tagmask) != selmon->tagset[selmon->seltags]) {
        view(arg);
        return;
    }

    int x, y, tagx = 0;
    int leftbar = 0;
    XMotionEvent last_motion = {0};
    XMotionEvent *last_motion_ptr = NULL;

    if (!selmon->sel)
        return;

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurMove]->cursor, CurrentTime) != GrabSuccess)
        return;
    if (!getrootptr(&x, &y))
        return;
    bardragging = 1;

    DragtagData data = {
        .leftbar = &leftbar, .tagx = &tagx, .last_motion = NULL};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, dragtag_motion, NULL);

    /* Get final position from stored motion event */
    if (data.last_motion) {
        last_motion = *data.last_motion;
        last_motion_ptr = &last_motion;
    }

    if (!leftbar && last_motion_ptr) {
        if (last_motion_ptr->x_root < selmon->mx + tagwidth) {
            if (last_motion_ptr->state & ShiftMask) {
                followtag(
                    &((Arg){.ui = 1 << getxtag(last_motion_ptr->x_root)}));
            } else if (last_motion_ptr->state & ControlMask) {
                tagall(&((Arg){.ui = 1 << getxtag(last_motion_ptr->x_root)}));
            } else {
                tag(&((Arg){.ui = 1 << getxtag(last_motion_ptr->x_root)}));
            }
        } else if (last_motion_ptr->x_root >
                   selmon->mx + selmon->mw - OVERLAY_ZONE_WIDTH) {
            if (selmon->sel == selmon->overlay) {
                setoverlay(NULL);
            } else {
                createoverlay(NULL);
                selmon->gesture = GestureOverlay;
            }
        }
    }
    bardragging = 0;
    XUngrabPointer(dpy, CurrentTime);
}

void forceresizemouse(const Arg *arg) {
    forceresize = 1;
    resizemouse(arg);
    forceresize = 0;
}

static int get_resize_direction(Client *c, int nx, int ny) {
    if (ny > c->h / 2) {     // bottom
        if (nx < c->w / 3) { // left
            if (ny < 2 * c->h / 3)
                return 7; // side
            else
                return 6;               // corner
        } else if (nx > 2 * c->w / 3) { // right
            if (ny < 2 * c->h / 3)
                return 3; // side
            else
                return 4; // corner
        } else {
            // middle
            return 5;
        }
    } else {                 // top
        if (nx < c->w / 3) { // left
            if (ny > c->h / 3)
                return 7; // side
            else
                return 0;               // corner
        } else if (nx > 2 * c->w / 3) { // right
            if (ny > c->h / 3)
                return 3; // side
            else
                return 2; // corner
        } else {
            // cursor on middle
            return 1;
        }
    }
}

static Cursor get_resize_cursor(int corner) {
    switch (corner) {
    case 0:
        return cursor[CurTL]->cursor;
    case 1:
        return cursor[CurVert]->cursor;
    case 2:
        return cursor[CurTR]->cursor;
    case 3:
        return cursor[CurHor]->cursor;
    case 4:
        return cursor[CurBR]->cursor;
    case 5:
        return cursor[CurVert]->cursor;
    case 6:
        return cursor[CurBL]->cursor;
    case 7:
        return cursor[CurHor]->cursor;
    default:
        return cursor[CurMove]->cursor;
    }
}

static void warp_pointer_resize(Client *c, int corner) {
    int x_off, y_off;
    switch (corner) {
    case 0:
        x_off = -c->bw;
        y_off = -c->bw;
        break;
    case 1:
        x_off = (c->w + c->bw - 1) / 2;
        y_off = -c->bw;
        break;
    case 2:
        x_off = c->w + c->bw - 1;
        y_off = -c->bw;
        break;
    case 3:
        x_off = c->w + c->bw - 1;
        y_off = (c->h + c->bw - 1) / 2;
        break;
    case 4:
        x_off = c->w + c->bw - 1;
        y_off = c->h + c->bw - 1;
        break;
    case 5:
        x_off = (c->w + c->bw - 1) / 2;
        y_off = c->h + c->bw - 1;
        break;
    case 6:
        x_off = -c->bw;
        y_off = c->h + c->bw - 1;
        break;
    case 7:
        x_off = -c->bw;
        y_off = (c->h + c->bw - 1) / 2;
        break;
    default:
        return;
    }
    XWarpPointer(dpy, None, c->win, 0, 0, 0, 0, x_off, y_off);
}

static void calc_resize_geometry(Client *c, XEvent *ev, int corner, int ocx,
                                 int ocy, int ocx2, int ocy2, int *nx, int *ny,
                                 int *nw, int *nh) {
    int horizcorner = (corner == 0 || corner == 6 || corner == 7);
    int vertcorner = (corner == 0 || corner == 1 || corner == 2);

    if (corner != 1 && corner != 5) {
        *nx = horizcorner ? ev->xmotion.x : c->x;
        *nw = MAX(horizcorner ? (ocx2 - *nx)
                              : (ev->xmotion.x - ocx - 2 * c->bw + 1),
                  1);
    } else {
        *nx = c->x;
        *nw = c->w;
    }

    //TODO get rid of magic numbers, use existing enum if possible, otherwise create new enum
    if (corner != 7 && corner != 3) {
        *ny = vertcorner ? ev->xmotion.y : c->y;
        *nh = MAX(vertcorner ? (ocy2 - *ny)
                             : (ev->xmotion.y - ocy - 2 * c->bw + 1),
                  1);
    } else {
        *ny = c->y;
        *nh = c->h;
    }
}

/* Data structure for resizemouse motion handler */
typedef struct {
    Client *c;
    int ocx, ocy, ocx2, ocy2;
    int corner;
} ResizemouseData;

static DragResult resizemouse_motion(XEvent *ev, void *data) {
    ResizemouseData *d = (ResizemouseData *)data;
    Client *c = d->c;
    int nx, ny, nw, nh;

    calc_resize_geometry(c, ev, d->corner, d->ocx, d->ocy, d->ocx2, d->ocy2,
                         &nx, &ny, &nw, &nh);

    if (c->mon->wx + nw >= selmon->wx &&
        c->mon->wx + nw <= selmon->wx + selmon->ww &&
        c->mon->wy + nh >= selmon->wy &&
        c->mon->wy + nh <= selmon->wy + selmon->wh) {
        if (!c->isfloating && tiling_layout_func(selmon) &&
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
    if (!tiling_layout_func(selmon) || c->isfloating) {
        if (c->bw == 0 && c != selmon->overlay)
            c->bw = c->oldbw;
        if (!forceresize)
            resize(c, nx, ny, nw, nh, 1);
        else
            resizeclient(c, nx, ny, nw, nh);
    }

    return DRAG_CONTINUE;
}

void resizemouse(const Arg *arg) {
    int nx, ny;
    Client *c;
    int corner;
    int di;
    unsigned int dui;
    Window dummy;

    if (!(c = selmon->sel))
        return;

    if (c == selmon->fullscreen) {
        tempfullscreen(NULL);
        return;
    }

    if (c->isfullscreen &&
        !c->isfakefullscreen) /* no support resizing fullscreen windows by mouse
                               */
        return;

    restack(selmon);

    if (!XQueryPointer(dpy, c->win, &dummy, &dummy, &di, &di, &nx, &ny, &dui))
        return;

    corner = get_resize_direction(c, nx, ny);

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, get_resize_cursor(corner),
                     CurrentTime) != GrabSuccess)
        return;

    warp_pointer_resize(c, corner);

    ResizemouseData data = {.c = c,
                            .ocx = c->x,
                            .ocy = c->y,
                            .ocx2 = c->x + c->w,
                            .ocy2 = c->y + c->h,
                            .corner = corner};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, resizemouse_motion, NULL);

    XUngrabPointer(dpy, CurrentTime);
    XEvent ev;
    while (XCheckMaskEvent(dpy, EnterWindowMask, &ev))
        ;
    handle_client_monitor_switch(c);

    if (NULL == tiling_layout_func(selmon)) {
        savefloating(c);
        c->snapstatus = SnapNone;
    }
}

/* Data structure for resizeaspectmouse motion handler */
typedef struct {
    Client *c;
    int ocx, ocy;
} ResizeaspectData;

static DragResult resizeaspect_motion(XEvent *ev, void *data) {
    ResizeaspectData *d = (ResizeaspectData *)data;
    Client *c = d->c;
    int nx, ny, nw, nh;

    nx = ev->xmotion.x;
    ny = ev->xmotion.y;

    if (abs(selmon->wx - nx) < snap)
        nx = selmon->wx;
    else if (abs((selmon->wx + selmon->ww) - (nx + WIDTH(c))) < snap)
        nx = selmon->wx + selmon->ww - WIDTH(c);
    if (abs(selmon->wy - ny) < snap)
        ny = selmon->wy;
    else if (abs((selmon->wy + selmon->wh) - (ny + HEIGHT(c))) < snap)
        ny = selmon->wy + selmon->wh - HEIGHT(c);

    if (!c->isfloating && tiling_layout_func(selmon) &&
        (abs(nx - c->x) > snap || abs(ny - c->y) > snap))
        togglefloating(NULL);
    if (!tiling_layout_func(selmon) || c->isfloating) {
        nw = MAX(nx - d->ocx - 2 * c->bw + 1, 1);
        nh = MAX(ny - d->ocy - 2 * c->bw + 1, 1);

        if (c->minw || c->minh) {
            if (nw < c->minw)
                nw = c->minw;
            if (nh < c->minh)
                nh = c->minh;
        }
        if (c->maxw || c->maxh) {
            if (nw > c->maxw)
                nw = c->maxw;
            if (nh > c->maxh)
                nh = c->maxh;
        }

        if (c->mina != 0.0 && c->maxa != 0.0) {
            if (c->maxa < (float)nw / nh)
                nw = nh * c->maxa;
            else if (c->mina < (float)nh / nw)
                nh = nw * c->mina;
        }

        resize(c, c->x, c->y, nw, nh, 1);
    }

    return DRAG_CONTINUE;
}

void resizeaspectmouse(const Arg *arg) {
    int nx, ny;
    Client *c;
    int di;
    unsigned int dui;
    Window dummy;

    if (!(c = selmon->sel))
        return;

    if (c->isfullscreen &&
        !c->isfakefullscreen) /* no support resizing fullscreen windows by mouse
                               */
        return;

    restack(selmon);

    if (!XQueryPointer(dpy, c->win, &dummy, &dummy, &di, &di, &nx, &ny, &dui))
        return;

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurResize]->cursor,
                     CurrentTime) != GrabSuccess)
        return;

    ResizeaspectData data = {.c = c, .ocx = c->x, .ocy = c->y};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, resizeaspect_motion, NULL);

    XUngrabPointer(dpy, CurrentTime);
    XEvent ev;
    while (XCheckMaskEvent(dpy, EnterWindowMask, &ev))
        ;
    handle_client_monitor_switch(c);
}

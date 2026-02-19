/* See LICENSE file for copyright and license details. */
#define _POSIX_C_SOURCE 200809L

#include <X11/XF86keysym.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bar.h"
#include "client.h"
#include "floating.h"
#include "focus.h"
#include "globals.h"
#include "instantwm.h"
#include "layouts.h"
#include "monitors.h"
#include "mouse.h"
#include "overlay.h"
#include "tags.h"
#include "util.h"

/* External declarations not covered by headers */
extern int force_resize;
extern void (*handler[LASTEvent])(XEvent *);
extern const char *upvol[];
extern const char *downvol[];

static void warp_pointer_resize(Client *c, int direction);
static int get_resize_direction(Client *c, int nx, int ny);

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

    if (ctx && ctx->extra_mask) {
        mask |= ctx->extra_mask;
    }

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
            if ((ev.xmotion.time - lasttime) <= (1000 / rate)) {
                continue;
            }
            lasttime = ev.xmotion.time;
            if (motion_handler) {
                if (motion_handler(&ev, ctx ? ctx->data : NULL) == DRAG_BREAK) {
                    return 0;
                }
            }
            break;
        default:
            if (extra_handler) {
                if (extra_handler(&ev, ctx ? ctx->data : NULL) == DRAG_BREAK) {
                    return 0;
                }
            }
            break;
        }
    } while (ev.type != ButtonRelease);

    return 1;
}

/* Handle window drop on bar: move to tag or re-tile */
static void handle_bar_drop(Client *c) {
    int x;
    int y;
    getrootptr(&x, &y);

    if (y < selmon->my || y >= selmon->my + bh) {
        return;
    }

    /* Check if dropped on a tag indicator */
    int droptag = get_tag_at_x(x);
    if (droptag >= 0 && x < selmon->mx + gettagwidth()) {
        /* Move window to that tag */
        tag(&((Arg){.ui = 1 << droptag}));

        /* Make floating windows tiled when dropped on tag indicator
         * Note: tag() calls focus(NULL) which changes selmon->sel,
         * so we must use settiled() which operates on the specific client
         */
        set_tiled(c, 1);
    } else if (c->isfloating) {
        /* Dropped elsewhere on bar - make it tiled again */
        toggle_floating(NULL);
    }
}

/* Data structure for movemouse motion handler */
typedef struct {
    Client *c;
    int ocx, ocy;            /* Original client position */
    int startx, starty;      /* Initial pointer position */
    int edge_snap_indicator; /* See Snap enum */
} MovemouseData;

/**
 * Snap coordinates to monitor work area edges if within snap threshold.
 *
 * Adjusts nx and ny to align with the monitor's work area boundaries
 * (selmon->wx, wy, ww, wh) if they are within the global snap threshold.
 * This provides consistent edge-snapping behavior during window move/resize.
 *
 * @param c   The client being moved/resized (used for WIDTH/HEIGHT macros)
 * @param nx  Pointer to x coordinate (modified in place)
 * @param ny  Pointer to y coordinate (modified in place)
 */
static void snap_to_monitor_edges(Client *c, int *nx, int *ny) {
    if (abs(selmon->wx - *nx) < snap) {
        *nx = selmon->wx;
    } else if (abs((selmon->wx + selmon->ww) - (*nx + WIDTH(c))) < snap) {
        *nx = selmon->wx + selmon->ww - WIDTH(c);
    }
    if (abs(selmon->wy - *ny) < snap) {
        *ny = selmon->wy;
    } else if (abs((selmon->wy + selmon->wh) - (*ny + HEIGHT(c))) < snap) {
        *ny = selmon->wy + selmon->wh - HEIGHT(c);
    }
}

/**
 * Check if cursor is at edge for snapping.
 * Returns: SnapNone, SnapLeft, SnapRight, or SnapTop
 */
static int check_edge_snap(int x, int y, Monitor *m) {
    if (x < m->mx + OVERLAY_ZONE_WIDTH && x > m->mx - 1) {
        return SnapLeft;
    }
    if (x > m->mx + m->mw - OVERLAY_ZONE_WIDTH && x < m->mx + m->mw + 1) {
        return SnapRight;
    }
    if (y <= m->my + (m->showbar ? bh : 5)) {
        return SnapTop;
    }
    return SnapNone;
}

static DragResult movemouse_motion(XEvent *ev, void *data) {
    MovemouseData *d = (MovemouseData *)data;
    Client *c = d->c;
    int nx;
    int ny;

    nx = d->ocx + (ev->xmotion.x - d->startx);
    ny = d->ocy + (ev->xmotion.y - d->starty);

    /* Check if cursor is at edge for snapping indicator */
    int at_edge =
        check_edge_snap(ev->xmotion.x_root, ev->xmotion.y_root, selmon);

    /* Update border color to indicate snap */
    if (at_edge && !d->edge_snap_indicator) {
        XSetWindowBorder(dpy, c->win, borderscheme[SchemeBorderSnap].pixel);
        d->edge_snap_indicator = at_edge;
    } else if (!at_edge && d->edge_snap_indicator) {
        XSetWindowBorder(dpy, c->win,
                         borderscheme[SchemeBorderFloatFocus].pixel);
        d->edge_snap_indicator = SnapNone;
    }

    /* If cursor is on the bar, offset window below the bar and update bar hover
     */
    if (ev->xmotion.y_root >= selmon->my &&
        ev->xmotion.y_root < selmon->my + bh) {
        ny = selmon->my + bh;
        if (!d->edge_snap_indicator) {
            XSetWindowBorder(dpy, c->win, borderscheme[SchemeBorderSnap].pixel);
            d->edge_snap_indicator = SnapTop;
        }
        /* Update bar hover state while dragging */
        bar_dragging = 1;
        if (!tagwidth) {
            tagwidth = gettagwidth();
        }
        if (ev->xmotion.x_root <
            selmon->mx + tagwidth + get_layout_symbol_width(selmon)) {
            /* Over tags area - could update tag hover */
            int tag = get_tag_at_x(ev->xmotion.x_root);
            if (tag >= 0 && tag < numtags) {
                if (selmon->gesture != tag + 1) {
                    selmon->gesture = tag + 1;
                    drawbar(selmon);
                }
            }
        } else if (ev->xmotion.x_root <
                   selmon->mx + get_layout_symbol_width(selmon) + tagwidth +
                       selmon->bar_clients_width) {
            /* Over window titles area */
            resetbar();
        }
    } else if (d->edge_snap_indicator == SnapTop &&
               ev->xmotion.y_root >= selmon->my + bh) {
        /* Left top bar area */
        XSetWindowBorder(dpy, c->win,
                         borderscheme[SchemeBorderFloatFocus].pixel);
        d->edge_snap_indicator = SnapNone;
        bar_dragging = 0;
    }

    snap_to_monitor_edges(c, &nx, &ny);
    if (!c->isfloating && tiling_layout_func(selmon) &&
        (abs(nx - c->x) > snap || abs(ny - c->y) > snap)) {
        if (animated) {
            animated = 0;
            toggle_floating(NULL);
            animated = 1;
        } else {
            toggle_floating(NULL);
        }
    }
    if (!tiling_layout_func(selmon) || c->isfloating) {
        resize(c, nx, ny, c->w, c->h, 1);
    }

    return DRAG_CONTINUE;
}

void movemouse(const Arg *arg) {
    int x;
    int y;
    Client *c;

    // some windows are immovable
    if (!(c = selmon->sel) || (c->is_fullscreen && !c->isfakefullscreen) ||
        c == selmon->overlay) {
        return;
    }

    if (c == selmon->fullscreen) {
        temp_fullscreen(NULL);
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
                     None, cursor[CurMove]->cursor,
                     CurrentTime) != GrabSuccess) {
        return;
    }
    if (!getrootptr(&x, &y)) {
        return;
    }

    MovemouseData data = {.c = c,
                          .ocx = c->x,
                          .ocy = c->y,
                          .startx = x,
                          .starty = y,
                          .edge_snap_indicator = SnapNone};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, movemouse_motion, NULL);
    XUngrabPointer(dpy, CurrentTime);
    bar_dragging = 0;

    /* Handle edge snapping on release */
    if (data.edge_snap_indicator) {
        int rootx;
        int rooty;
        getrootptr(&rootx, &rooty);

        /* Query current button state to check for Shift */
        Window dummy_win;
        int dummy_int;
        unsigned int button_state;
        XQueryPointer(dpy, root, &dummy_win, &dummy_win, &dummy_int, &dummy_int,
                      &dummy_int, &dummy_int, &button_state);

        int snap_direction = check_edge_snap(rootx, rooty, selmon);
        int at_left_edge = (snap_direction == SnapLeft);
        int at_right_edge = (snap_direction == SnapRight);
        int at_top_edge = (snap_direction == SnapTop);

        if (at_left_edge || at_right_edge) {
            if (button_state & ShiftMask ||
                NULL == tiling_layout_func(selmon)) {
                /* Shift held or floating layout: snap to half/quarter screen */
                XSetWindowBorder(dpy, c->win,
                                 borderscheme[SchemeBorderTileFocus].pixel);
                savefloating(c);

                if (at_right_edge) {
                    if (rooty < selmon->my + selmon->mh / 7) {
                        c->snapstatus = SnapTopRight;
                    } else if (rooty > selmon->my + 6 * (selmon->mh / 7)) {
                        c->snapstatus = SnapBottomRight;
                    } else {
                        c->snapstatus = SnapRight;
                    }
                } else {
                    if (rooty < selmon->my + selmon->mh / 7) {
                        c->snapstatus = SnapTopLeft;
                    } else if (rooty > selmon->my + 6 * (selmon->mh / 7)) {
                        c->snapstatus = SnapBottomLeft;
                    } else {
                        c->snapstatus = SnapLeft;
                    }
                }
                applysnap(c, c->mon);
            } else {
                /* No shift: move to adjacent tag */
                if (rooty < selmon->my + (2 * selmon->mh) / 3) {
                    if (at_left_edge) {
                        moveleft(NULL);
                    } else {
                        moveright(NULL);
                    }
                } else {
                    if (at_left_edge) {
                        tagtoleft(NULL);
                    } else {
                        tagtoright(NULL);
                    }
                }
                c->isfloating = 0;
                arrange(selmon);
            }
            return;
        }
        if (at_top_edge) {
            /* Reset border and continue to normal bar drop handling */
            XSetWindowBorder(dpy, c->win,
                             borderscheme[SchemeBorderFloatFocus].pixel);
        }
    }

    /* Reset border if needed */
    if (c->isfloating) {
        XSetWindowBorder(dpy, c->win,
                         borderscheme[SchemeBorderFloatFocus].pixel);
    }

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
        if (ev->xmotion.y_root < *d->lasty) {
            spawn(&((Arg){.v = upvol}));
        } else {
            spawn(&((Arg){.v = downvol}));
        }
        *d->lasty = ev->xmotion.y_root;
    }

    return DRAG_CONTINUE;
}

void gesturemouse(const Arg *arg) {
    int x;
    int y;
    int lasty;

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurMove]->cursor,
                     CurrentTime) != GrabSuccess) {
        return;
    }
    if (!getrootptr(&x, &y)) {
        return;
    }
    lasty = y;

    GesturemouseData data = {.lasty = &lasty};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, gesturemouse_motion, NULL);
    XUngrabPointer(dpy, CurrentTime);
}

// Check if cursor is in the resize border zone around the selected floating
// window Returns 1 if in border zone, 0 if not or no valid floating selection
int is_in_resize_border() {
    if (!(selmon->sel &&
          (selmon->sel->isfloating || !tiling_layout_func(selmon)))) {
        return 0;
    }
    int x;
    int y;
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

/* Data structure for hover_resize_mouse handlers */
typedef struct {
    int inborder;
    int resize_started;
} HoverResizeData;

static DragResult hoverresize_motion(XEvent *ev, void *data) {
    HoverResizeData *d = (HoverResizeData *)data;

    if (!is_in_resize_border()) {
        d->inborder = 0;
        Client *newc = getcursorclient();
        if (newc && newc != selmon->sel) {
            focus(newc);
        }
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
            Client *c = selmon->sel;
            if (c) {
                int nx;
                int ny;
                int di;
                unsigned int dui;
                Window dummy;
                /* Check if the click is on the top edge of the window.
                 * If so, we treat it as a move operation instead of a resize.
                 * This emulates grabbing a window title bar for windows that do
                 * not have one (e.g. terminals, or when decorations are
                 * disabled), using the hover behavior to detect the "border".
                 */
                if (XQueryPointer(dpy, c->win, &dummy, &dummy, &di, &di, &nx,
                                  &ny, &dui)) {
                    int direction = get_resize_direction(c, nx, ny);
                    if (direction == ResizeDirTop) {
                        XUngrabPointer(dpy, CurrentTime);
                        /* Warp pointer to the top edge to initiate the move
                         * from a predictable point, consistent with
                         * Super+RightClick move behavior.
                         */
                        warp_into(c);
                        movemouse(NULL);
                        d->resize_started = 1;
                        return DRAG_BREAK;
                    }
                }
            }
            XUngrabPointer(dpy, CurrentTime);
            resizemouse(NULL);
            d->resize_started = 1;
            return DRAG_BREAK;
        } else if (ev->xbutton.button == Button3) {
            /* Right click in the resize border triggers a move operation.
             * This provides an alternative way to move windows without grabbing
             * the title bar.
             */
            Client *c = selmon->sel;
            XUngrabPointer(dpy, CurrentTime);
            if (c) {
                warp_into(c);
            }
            movemouse(NULL);
            d->resize_started = 1;
            return DRAG_BREAK;
        }
        break;
    }

    return DRAG_CONTINUE;
}

int hover_resize_mouse(const Arg *arg) {
    if (!is_in_resize_border()) {
        return 0;
    }

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurResize]->cursor,
                     CurrentTime) != GrabSuccess) {
        return 0;
    }

    HoverResizeData data = {.inborder = 1, .resize_started = 0};
    DragContext ctx = {.extra_mask = KeyPressMask, .data = &data};

    drag_loop(&ctx, hoverresize_motion, hoverresize_extra);

    if (!data.resize_started) {
        XUngrabPointer(dpy, CurrentTime);
    }
    return 1;
}

static void drag_window_title(Client *c) {
    XUngrabPointer(dpy, CurrentTime);
    show(c);
    focus(c);
    restack(selmon);
    if (selmon->sel) {
        warp_into(selmon->sel);
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
    int x;
    int y;

    getrootptr(&x, &y);
    /* If mouse moved beyond threshold, start moving the window */
    if (abs(x - d->startx) > DRAG_THRESHOLD ||
        abs(y - d->starty) > DRAG_THRESHOLD) {
        if (d->was_hidden) {
            show(d->c);
        }
        drag_window_title(d->c);
        d->drag_started = 1;
        return DRAG_BREAK;
    }

    return DRAG_CONTINUE;
}

void window_title_mouse_handler(const Arg *arg) {
    int startx;
    int starty;

    Client *c = (Client *)arg->v;
    if (!c) {
        return;
    }

    int was_focused = (selmon->sel == c);
    int was_hidden = HIDDEN(c);

    /* Grab pointer to detect if user drags or just clicks */
    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurNormal]->cursor,
                     CurrentTime) != GrabSuccess) {
        return;
    }
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
    if (data.drag_started) {
        return;
    }

    /* Button released without significant movement - just focus */
    XUngrabPointer(dpy, CurrentTime);
    click_window_title(c, was_hidden, was_focused);
}

/* Data structure for window_title_mouse_handler_right motion handler */
typedef struct {
    Client *c;
    int startx, starty;
    int start_initialized;
    int dragging;
} RightTitleData;

static DragResult right_title_motion(XEvent *ev, void *data) {
    RightTitleData *d = (RightTitleData *)data;
    XMotionEvent *motion = &ev->xmotion;

    if (!d->start_initialized) {
        d->startx = motion->x_root;
        d->starty = motion->y_root;
        d->start_initialized = 1;
    }

    if (abs(motion->x_root - d->startx) > DRAG_THRESHOLD ||
        abs(motion->y_root - d->starty) > DRAG_THRESHOLD) {
        d->dragging = 1;
        XUngrabPointer(dpy, CurrentTime);

        if (HIDDEN(d->c)) {
            show(d->c);
            focus(d->c);
            restack(selmon);
        }

        warp_pointer_resize(d->c, ResizeDirBottomRight);
        resizemouse(NULL);
        return DRAG_BREAK;
    }

    return DRAG_CONTINUE;
}

void window_title_mouse_handler_right(const Arg *arg) {
    int x;
    int y;
    Client *c = (Client *)arg->v;
    if (!c) {
        return;
    }

    resetbar();
    if (c->is_fullscreen &&
        !c->isfakefullscreen) { /* no support moving fullscreen windows by
                                     mouse */
        return;
    }

    focus(c);

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurMove]->cursor,
                     CurrentTime) != GrabSuccess) {
        return;
    }
    if (!getrootptr(&x, &y)) {
        XUngrabPointer(dpy, CurrentTime);
        return;
    }

    RightTitleData data = {.c = c,
                           .startx = x,
                           .starty = y,
                           .start_initialized = 1,
                           .dragging = 0};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, right_title_motion, NULL);

    if (!data.dragging) {
        XUngrabPointer(dpy, CurrentTime);
        if (HIDDEN(c)) {
            show(c);
            focus(c);
            restack(selmon);
        }
        zoom(NULL);
    }
}

/* Helper functions for drawwindow */
int parse_slop_output(const char *output, int dimensions[4]) {
    char strout[100] = {0};
    char tmpstring[30] = {0};
    int firstchar = 0;
    int counter = 0;
    int i;

    if (!output || strlen(output) < 6) {
        return 0;
    }

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
        toggle_floating(NULL);
        resize(c, x, y, width, height, 1);
    }
}

void drawwindow(const Arg *arg) {
    char str[100];
    char strout[100] = {0};
    int dimensions[4];
    int width;
    int height;
    int x;
    int y;
    Client *c;
    FILE *fp;

    if (!selmon->sel) {
        return;
    }

    fp = popen("instantslop -f x%xx%yx%wx%hx", "r");
    if (!fp) {
        return;
    }

    while (fgets(str, 100, fp) != NULL) {
        strcat(strout, str);
    }
    pclose(fp);

    if (!parse_slop_output(strout, dimensions)) {
        return;
    }

    x = dimensions[0];
    y = dimensions[1];
    width = dimensions[2];
    height = dimensions[3];

    if (!selmon->sel) {
        return;
    }

    c = selmon->sel;

    if (is_valid_window_size(x, y, width, height, c)) {
        handle_monitor_switch(c, x, y, width, height);
        apply_window_resize(c, x, y, width, height);
    }
}

/* Data structure for dragtag motion handler */
typedef struct {
    int *cursor_on_bar;
    int *tagx;
    XMotionEvent *last_motion; /* Store last motion for post-loop processing */
} DragtagData;

static DragResult dragtag_motion(XEvent *ev, void *data) {
    DragtagData *d = (DragtagData *)data;

    /* Store the motion event for post-loop use */
    d->last_motion = &ev->xmotion;

    if (ev->xmotion.y_root > selmon->my + bh + 1) {
        *d->cursor_on_bar = 0;
    }

    int newtag = get_tag_at_x(ev->xmotion.x_root);
    if (*d->tagx != newtag) {
        *d->tagx = newtag;
        selmon->gesture = newtag + 1;
        drawbar(selmon);
    }

    return *d->cursor_on_bar ? DRAG_CONTINUE : DRAG_BREAK;
}

void dragtag(const Arg *arg) {
    if (!tagwidth) {
        tagwidth = gettagwidth();
    }
    if ((arg->ui & tagmask) != selmon->tagset[selmon->seltags]) {
        view(arg);
        return;
    }

    int x;
    int y;
    int tagx = 0;
    int cursor_on_bar = 1;
    XMotionEvent last_motion = {0};
    XMotionEvent *last_motion_ptr = NULL;

    if (!selmon->sel) {
        return;
    }

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurMove]->cursor,
                     CurrentTime) != GrabSuccess) {
        return;
    }
    if (!getrootptr(&x, &y)) {
        return;
    }
    bar_dragging = 1;

    DragtagData data = {
        .cursor_on_bar = &cursor_on_bar, .tagx = &tagx, .last_motion = NULL};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, dragtag_motion, NULL);

    /* Get final position from stored motion event */
    if (data.last_motion) {
        last_motion = *data.last_motion;
        last_motion_ptr = &last_motion;
    }

    if (cursor_on_bar && last_motion_ptr) {
        if (last_motion_ptr->x_root < selmon->mx + tagwidth) {
            if (last_motion_ptr->state & ShiftMask) {
                followtag(
                    &((Arg){.ui = 1 << get_tag_at_x(last_motion_ptr->x_root)}));
            } else if (last_motion_ptr->state & ControlMask) {
                tagall(
                    &((Arg){.ui = 1 << get_tag_at_x(last_motion_ptr->x_root)}));
            } else {
                tag(&((Arg){.ui = 1 << get_tag_at_x(last_motion_ptr->x_root)}));
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
    bar_dragging = 0;
    XUngrabPointer(dpy, CurrentTime);
}

void forceresizemouse(const Arg *arg) {
    force_resize = 1;
    resizemouse(arg);
    force_resize = 0;
}

static int get_resize_direction(Client *c, int nx, int ny) {
    if (ny > c->h / 2) {     // bottom
        if (nx < c->w / 3) { // left
            if (ny < 2 * c->h / 3) {
                return ResizeDirLeft;
            }
            return ResizeDirBottomLeft;
        }
        if (nx > 2 * c->w / 3) { // right
            if (ny < 2 * c->h / 3) {
                return ResizeDirRight;
            }
            return ResizeDirBottomRight;
        }
        // middle
        return ResizeDirBottom;
    }
    // top
    if (nx < c->w / 3) { // left
        if (ny > c->h / 3) {
            return ResizeDirLeft;
        }
        return ResizeDirTopLeft;
    }
    if (nx > 2 * c->w / 3) { // right
        if (ny > c->h / 3) {
            return ResizeDirRight;
        }
        return ResizeDirTopRight;
    }
    // cursor on middle
    return ResizeDirTop;
}

static Cursor get_resize_cursor(int direction) {
    switch (direction) {
    case ResizeDirTopLeft:
        return cursor[CurTL]->cursor;
    case ResizeDirTop:
        return cursor[CurVert]->cursor;
    case ResizeDirTopRight:
        return cursor[CurTR]->cursor;
    case ResizeDirRight:
        return cursor[CurHor]->cursor;
    case ResizeDirBottomRight:
        return cursor[CurBR]->cursor;
    case ResizeDirBottom:
        return cursor[CurVert]->cursor;
    case ResizeDirBottomLeft:
        return cursor[CurBL]->cursor;
    case ResizeDirLeft:
        return cursor[CurHor]->cursor;
    default:
        return cursor[CurMove]->cursor;
    }
}

static void warp_pointer_resize(Client *c, int direction) {
    int x_off;
    int y_off;
    switch (direction) {
    case ResizeDirTopLeft:
        x_off = -c->border_width;
        y_off = -c->border_width;
        break;
    case ResizeDirTop:
        x_off = (c->w + c->border_width - 1) / 2;
        y_off = -c->border_width;
        break;
    case ResizeDirTopRight:
        x_off = c->w + c->border_width - 1;
        y_off = -c->border_width;
        break;
    case ResizeDirRight:
        x_off = c->w + c->border_width - 1;
        y_off = (c->h + c->border_width - 1) / 2;
        break;
    case ResizeDirBottomRight:
        x_off = c->w + c->border_width - 1;
        y_off = c->h + c->border_width - 1;
        break;
    case ResizeDirBottom:
        x_off = (c->w + c->border_width - 1) / 2;
        y_off = c->h + c->border_width - 1;
        break;
    case ResizeDirBottomLeft:
        x_off = -c->border_width;
        y_off = c->h + c->border_width - 1;
        break;
    case ResizeDirLeft:
        x_off = -c->border_width;
        y_off = (c->h + c->border_width - 1) / 2;
        break;
    default:
        return;
    }
    XWarpPointer(dpy, None, c->win, 0, 0, 0, 0, x_off, y_off);
}

/**
 * Calculate new window geometry during a resize operation.
 *
 * Based on the resize direction and current mouse position, this function
 * computes the new x, y, width, and height for the window. For edge-only
 * directions (Top, Bottom, Left, Right), only the relevant dimension changes.
 * For corner directions (TopLeft, TopRight, BottomLeft, BottomRight), both
 * dimensions change. The original edge positions are used to calculate
 * the new size relative to the fixed corner.
 *
 * @param c           The client being resized
 * @param ev          The motion event containing current mouse position
 * @param direction   The resize direction (ResizeDirTopLeft, ResizeDirBottom,
 * etc.)
 * @param orig_left   Original client x position (left edge)
 * @param orig_top    Original client y position (top edge)
 * @param orig_right  Original client right edge (x + width)
 * @param orig_bottom Original client bottom edge (y + height)
 * @param nx          Output: new x position
 * @param ny          Output: new y position
 * @param nw          Output: new width
 * @param nh          Output: new height
 */
static void calc_resize_geometry(Client *c, XEvent *ev, int direction,
                                 int orig_left, int orig_top, int orig_right,
                                 int orig_bottom, int *nx, int *ny, int *nw,
                                 int *nh) {
    int is_left_side =
        (direction == ResizeDirTopLeft || direction == ResizeDirBottomLeft ||
         direction == ResizeDirLeft);
    int is_top_side =
        (direction == ResizeDirTopLeft || direction == ResizeDirTop ||
         direction == ResizeDirTopRight);

    if (direction != ResizeDirTop && direction != ResizeDirBottom) {
        *nx = is_left_side ? ev->xmotion.x : c->x;
        *nw = MAX(is_left_side
                      ? (orig_right - *nx)
                      : (ev->xmotion.x - orig_left - 2 * c->border_width + 1),
                  1);
    } else {
        *nx = c->x;
        *nw = c->w;
    }

    if (direction != ResizeDirLeft && direction != ResizeDirRight) {
        *ny = is_top_side ? ev->xmotion.y : c->y;
        *nh = MAX(is_top_side
                      ? (orig_bottom - *ny)
                      : (ev->xmotion.y - orig_top - 2 * c->border_width + 1),
                  1);
    } else {
        *ny = c->y;
        *nh = c->h;
    }
}

/**
 * Check if a window with the given dimensions would be within the selected
 * monitor's work area bounds.
 *
 * This is used during resize operations to determine if the window should
 * automatically toggle to floating mode (when dragged outside tiled area).
 *
 * @param c   The client being resized
 * @param nw  The new width to check
 * @param nh  The new height to check
 * @return 1 if within bounds, 0 otherwise
 */
static int is_within_monitor_bounds(Client *c, int nw, int nh) {
    return (c->mon->wx + nw >= selmon->wx &&
            c->mon->wx + nw <= selmon->wx + selmon->ww &&
            c->mon->wy + nh >= selmon->wy &&
            c->mon->wy + nh <= selmon->wy + selmon->wh);
}

/**
 * Clamp width and height values to respect a client's size hints.
 *
 * Applies minimum and maximum size constraints from the client's size hints.
 * This is a simplified version of the clamping done in applysizehints(),
 * intended for use during interactive resize operations.
 *
 * @param c   The client whose size hints to use
 * @param nw  Pointer to width value (modified in place)
 * @param nh  Pointer to height value (modified in place)
 */
static void clamp_size_hints(Client *c, int *nw, int *nh) {
    if (c->minw && *nw < c->minw) {
        *nw = c->minw;
    }
    if (c->minh && *nh < c->minh) {
        *nh = c->minh;
    }
    if (c->maxw && *nw > c->maxw) {
        *nw = c->maxw;
    }
    if (c->maxh && *nh > c->maxh) {
        *nh = c->maxh;
    }
}

/* Data structure for resizemouse motion handler */
typedef struct {
    Client *c;
    int orig_left;   /* Original client x position (left edge) */
    int orig_top;    /* Original client y position (top edge) */
    int orig_right;  /* Original client right edge (x + width) */
    int orig_bottom; /* Original client bottom edge (y + height) */
    int corner;
} ResizemouseData;

static DragResult resizemouse_motion(XEvent *ev, void *data) {
    ResizemouseData *d = (ResizemouseData *)data;
    Client *c = d->c;
    int nx;
    int ny;
    int nw;
    int nh;

    calc_resize_geometry(c, ev, d->corner, d->orig_left, d->orig_top,
                         d->orig_right, d->orig_bottom, &nx, &ny, &nw, &nh);

    if (is_within_monitor_bounds(c, nw, nh)) {
        if (!c->isfloating && tiling_layout_func(selmon) &&
            (abs(nw - c->w) > snap || abs(nh - c->h) > snap)) {
            if (animated) {
                animated = 0;
                toggle_floating(NULL);
                animated = 1;
            } else {
                toggle_floating(NULL);
            }
        }
    }
    if (!tiling_layout_func(selmon) || c->isfloating) {
        if (c->border_width == 0 && c != selmon->overlay) {
            c->border_width = c->old_border_width;
        }
        if (!force_resize) {
            resize(c, nx, ny, nw, nh, 1);
        } else {
            resizeclient(c, nx, ny, nw, nh);
        }
    }

    return DRAG_CONTINUE;
}

void resizemouse(const Arg *arg) {
    int nx;
    int ny;
    Client *c;
    int corner;
    int di;
    unsigned int dui;
    Window dummy;

    if (!(c = selmon->sel)) {
        return;
    }

    if (c == selmon->fullscreen) {
        temp_fullscreen(NULL);
        return;
    }

    if (c->is_fullscreen &&
        !c->isfakefullscreen) { /* no support resizing fullscreen windows by
                                 * mouse
                                 */
        return;
    }

    restack(selmon);

    if (!XQueryPointer(dpy, c->win, &dummy, &dummy, &di, &di, &nx, &ny, &dui)) {
        return;
    }

    corner = get_resize_direction(c, nx, ny);

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, get_resize_cursor(corner),
                     CurrentTime) != GrabSuccess) {
        return;
    }

    warp_pointer_resize(c, corner);

    ResizemouseData data = {.c = c,
                            .orig_left = c->x,
                            .orig_top = c->y,
                            .orig_right = c->x + c->w,
                            .orig_bottom = c->y + c->h,
                            .corner = corner};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, resizemouse_motion, NULL);

    XUngrabPointer(dpy, CurrentTime);
    XEvent ev;
    while (XCheckMaskEvent(dpy, EnterWindowMask, &ev)) {
        ;
    }
    handle_client_monitor_switch(c);

    if (NULL == tiling_layout_func(selmon)) {
        savefloating(c);
        c->snapstatus = SnapNone;
    }
}

/* Data structure for resizeaspectmouse motion handler */
typedef struct {
    Client *c;
    int orig_left; /* Original client x position (left edge) */
    int orig_top;  /* Original client y position (top edge) */
} ResizeaspectData;

static DragResult resizeaspect_motion(XEvent *ev, void *data) {
    ResizeaspectData *d = (ResizeaspectData *)data;
    Client *c = d->c;
    int nx;
    int ny;
    int nw;
    int nh;

    nx = ev->xmotion.x;
    ny = ev->xmotion.y;

    snap_to_monitor_edges(c, &nx, &ny);

    if (!c->isfloating && tiling_layout_func(selmon) &&
        (abs(nx - c->x) > snap || abs(ny - c->y) > snap)) {
        toggle_floating(NULL);
    }
    if (!tiling_layout_func(selmon) || c->isfloating) {
        nw = MAX(nx - d->orig_left - 2 * c->border_width + 1, 1);
        nh = MAX(ny - d->orig_top - 2 * c->border_width + 1, 1);

        clamp_size_hints(c, &nw, &nh);

        if (c->mina != 0.0 && c->maxa != 0.0) {
            if (c->maxa < (float)nw / nh) {
                nw = nh * c->maxa;
            } else if (c->mina < (float)nh / nw) {
                nh = nw * c->mina;
            }
        }

        resize(c, c->x, c->y, nw, nh, 1);
    }

    return DRAG_CONTINUE;
}

void resizeaspectmouse(const Arg *arg) {
    int nx;
    int ny;
    Client *c;
    int di;
    unsigned int dui;
    Window dummy;

    if (!(c = selmon->sel)) {
        return;
    }

    if (c->is_fullscreen &&
        !c->isfakefullscreen) { /* no support resizing fullscreen windows by
                                 * mouse
                                 */
        return;
    }

    restack(selmon);

    if (!XQueryPointer(dpy, c->win, &dummy, &dummy, &di, &di, &nx, &ny, &dui)) {
        return;
    }

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurResize]->cursor,
                     CurrentTime) != GrabSuccess) {
        return;
    }

    ResizeaspectData data = {.c = c, .orig_left = c->x, .orig_top = c->y};
    DragContext ctx = {.data = &data};

    drag_loop(&ctx, resizeaspect_motion, NULL);

    XUngrabPointer(dpy, CurrentTime);
    XEvent ev;
    while (XCheckMaskEvent(dpy, EnterWindowMask, &ev)) {
        ;
    }
    handle_client_monitor_switch(c);
}

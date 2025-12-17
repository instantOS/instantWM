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

void movemouse(const Arg *arg) {
    int x, y, ocx, ocy, nx, ny, ti, tx, occ, colorclient, tagx, notfloating;
    Client *c;
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
        tempfullscreen(NULL);
        return;
    }

    if (c->snapstatus) {
        resetsnap(c);
        return;
    }

    if (NULL == selmon->lt[selmon->sellt]->arrange) {
        // unmaximize in floating layout
        if (c->x >= selmon->mx - MAX_UNMAXIMIZE_OFFSET &&
            c->y >= selmon->my + bh - MAX_UNMAXIMIZE_OFFSET &&
            c->w >= selmon->mw - MAX_UNMAXIMIZE_OFFSET &&
            c->h >= selmon->mh - MAX_UNMAXIMIZE_OFFSET) {
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
                (1000 / (doubledraw ? REFRESH_RATE_HI : REFRESH_RATE_LO)))
                continue;
            lasttime = ev.xmotion.time;

            nx = ocx + (ev.xmotion.x - x);
            ny = ocy + (ev.xmotion.y - y);

            /* If cursor is on the bar, offset window below the bar */
            if (ev.xmotion.y_root >= selmon->my &&
                ev.xmotion.y_root < selmon->my + bh) {
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
            if (!c->isfloating && selmon->lt[selmon->sellt]->arrange &&
                (abs(nx - c->x) > snap || abs(ny - c->y) > snap)) {
                if (animated) {
                    animated = 0;
                    togglefloating(NULL);
                    animated = 1;
                } else {
                    togglefloating(NULL);
                }
            }
            if (!selmon->lt[selmon->sellt]->arrange || c->isfloating)
                resize(c, nx, ny, c->w, c->h, 1);
            break;
        }
    } while (ev.type != ButtonRelease);
    XUngrabPointer(dpy, CurrentTime);

    /* Handle drop on bar (tag move, re-tile) */
    handle_bar_drop(c);

    handle_client_monitor_switch(c);
}

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
                (1000 / (doubledraw ? REFRESH_RATE_HI : REFRESH_RATE_LO)))
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
    XUngrabPointer(dpy, CurrentTime);
}

// Check if cursor is in the resize border zone around the selected floating
// window Returns 1 if in border zone, 0 if not or no valid floating selection
int isinresizeborder() {
    if (!(selmon->sel &&
          (selmon->sel->isfloating || !selmon->lt[selmon->sellt]->arrange)))
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

int hoverresizemouse(const Arg *arg) {
    if (!isinresizeborder())
        return 0;

    XEvent ev;
    Time lasttime = 0;
    int inborder = 1;

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
            if (ev.xkey.keycode == KEYCODE_ESCAPE) { // Escape key
                inborder = 0;
                break;
            }
            handler[ev.type](&ev);
            break;
        case ButtonPress:
            if (ev.xbutton.button == Button1) {
                XUngrabPointer(dpy, CurrentTime);
                resizemouse(NULL);
                return 1;
            }
            break;
        case MotionNotify:
            if ((ev.xmotion.time - lasttime) <=
                (1000 / (doubledraw ? REFRESH_RATE_HI : REFRESH_RATE_LO)))
                continue;
            lasttime = ev.xmotion.time;

            if (!isinresizeborder()) {
                inborder = 0;
                Client *newc = getcursorclient();
                if (newc && newc != selmon->sel)
                    focus(newc);
                break;
            }

        }
    } while (ev.type != ButtonRelease && inborder);

    XUngrabPointer(dpy, CurrentTime);
    return 1;
}

void dragmouse(const Arg *arg) {
    int x, y, startx, starty;
    XEvent ev;

    /* Focus the clicked window - use hoverclient since hover detection works
     * correctly */
    Client *c = selmon->hoverclient;
    if (c) {
        focus(c);
        restack(selmon);
    }

    /* Grab pointer to detect if user drags or just clicks */
    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurNormal]->cursor,
                     CurrentTime) != GrabSuccess)
        return;
    if (!getrootptr(&startx, &starty)) {
        XUngrabPointer(dpy, CurrentTime);
        return;
    }

    /* Wait for mouse movement or button release */
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
            getrootptr(&x, &y);
            /* If mouse moved beyond threshold, start moving the window */
            if (abs(x - startx) > DRAG_THRESHOLD ||
                abs(y - starty) > DRAG_THRESHOLD) {
                XUngrabPointer(dpy, CurrentTime);
                movemouse(NULL);
                return;
            }
            break;
        }
    } while (ev.type != ButtonRelease);

    /* Button released without significant movement - just focus (already done)
     */
    XUngrabPointer(dpy, CurrentTime);
}

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

    focus(tempc);

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurMove]->cursor, CurrentTime) != GrabSuccess)
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
            if ((ev.xmotion.time - lasttime) <= (1000 / REFRESH_RATE_DRAG))
                continue;
            lasttime = ev.xmotion.time;
            if (!sinit) {
                startx = ev.xmotion.x_root;
                starty = ev.xmotion.y_root;
                sinit = 1;
            }

            if (abs(ev.xmotion.x_root - startx) > selmon->mw / 20) {
                if (!dragging) {
                    Arg a = {.i = (ev.xmotion.x_root < startx) ? -1 : 1};
                    tagtoleft(&a);
                    dragging = 1;
                }
            } else if (abs(ev.xmotion.y_root - starty) > GESTURE_THRESHOLD) {
                // down
                if (ev.xmotion.y_root > starty) {
                    hidewin(NULL);
                } else {
                    killclient(NULL);
                }
                return;
            }

            break;
        }
    } while (ev.type != ButtonRelease);
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
            width < selmon->mw + SLOP_MARGIN && height < selmon->mh + SLOP_MARGIN &&
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

void dragtag(const Arg *arg) {
    if (!tagwidth)
        tagwidth = gettagwidth();
    if ((arg->ui & tagmask) != selmon->tagset[selmon->seltags]) {
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
            if ((ev.xmotion.time - lasttime) <= (1000 / REFRESH_RATE_DRAG))
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
        } else if (ev.xmotion.x_root >
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

void resizemouse(const Arg *arg) {
    int ocx, ocy, nw, nh;
    int ocx2, ocy2, nx, ny;
    Client *c;
    XEvent ev;
    int corner;
    int di;
    unsigned int dui;
    Window dummy;
    Time lasttime = 0;

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
    ocx = c->x;
    ocy = c->y;
    ocx2 = c->x + c->w;
    ocy2 = c->y + c->h;

    if (!XQueryPointer(dpy, c->win, &dummy, &dummy, &di, &di, &nx, &ny, &dui))
        return;

    corner = get_resize_direction(c, nx, ny);

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, get_resize_cursor(corner),
                     CurrentTime) != GrabSuccess)
        return;

    warp_pointer_resize(c, corner);

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
                (1000 / (doubledraw ? REFRESH_RATE_HI : REFRESH_RATE_LO)))
                continue;
            lasttime = ev.xmotion.time;

            calc_resize_geometry(c, &ev, corner, ocx, ocy, ocx2, ocy2, &nx, &ny,
                                 &nw, &nh);

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
    handle_client_monitor_switch(c);

    if (NULL == selmon->lt[selmon->sellt]->arrange) {
        savefloating(c);
        c->snapstatus = SnapNone;
    }
}

void resizeaspectmouse(const Arg *arg) {
    int ocx, ocy, nw, nh;
    int ocx2, ocy2, nx, ny;
    Client *c;
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
    ocx2 = c->x + c->w;
    ocy2 = c->y + c->h;

    if (!XQueryPointer(dpy, c->win, &dummy, &dummy, &di, &di, &nx, &ny, &dui))
        return;

    if (XGrabPointer(dpy, root, False, MOUSEMASK, GrabModeAsync, GrabModeAsync,
                     None, cursor[CurResize]->cursor,
                     CurrentTime) != GrabSuccess)
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
            if ((ev.xmotion.time - lasttime) <=
                (1000 / (doubledraw ? REFRESH_RATE_HI : REFRESH_RATE_LO)))
                continue;
            lasttime = ev.xmotion.time;

            nx = ev.xmotion.x;
            ny = ev.xmotion.y;

            if (abs(selmon->wx - nx) < snap)
                nx = selmon->wx;
            else if (abs((selmon->wx + selmon->ww) - (nx + WIDTH(c))) < snap)
                nx = selmon->wx + selmon->ww - WIDTH(c);
            if (abs(selmon->wy - ny) < snap)
                ny = selmon->wy;
            else if (abs((selmon->wy + selmon->wh) - (ny + HEIGHT(c))) < snap)
                ny = selmon->wy + selmon->wh - HEIGHT(c);

            if (!c->isfloating && selmon->lt[selmon->sellt]->arrange &&
                (abs(nx - c->x) > snap || abs(ny - c->y) > snap))
                togglefloating(NULL);
            if (!selmon->lt[selmon->sellt]->arrange || c->isfloating) {
                nw = MAX(nx - ocx - 2 * c->bw + 1, 1);
                nh = MAX(ny - ocy - 2 * c->bw + 1, 1);

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
            break;
        }
    } while (ev.type != ButtonRelease);

    XUngrabPointer(dpy, CurrentTime);
    while (XCheckMaskEvent(dpy, EnterWindowMask, &ev))
        ;
    handle_client_monitor_switch(c);
}

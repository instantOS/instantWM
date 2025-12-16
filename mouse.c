/* See LICENSE file for copyright and license details. */
#include "mouse.h"
#include "instantwm.h"
#include "layouts.h"
#include "util.h"

#include <X11/XF86keysym.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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
        tempfullscreen(NULL);
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
            ny = ocy + (ev.xmotion.y - y);
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
    if ((m = recttomon(c->x, c->y, c->w, c->h)) != selmon) {
        sendmon(c, m);
        unfocus(selmon->sel, 0);
        selmon = m;
        focus(NULL);
    }
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
        y < c->y - 30 || x < c->x - 30 || y > c->y + c->h + 30 ||
        x > c->x + c->w + 30) {
        return 0;
    }
    return 1;
}

int resizeborder(const Arg *arg) {
    if (!isinresizeborder())
        return 1;

    XEvent ev;
    Time lasttime = 0;
    Client *c = selmon->sel;
    int inborder = 1;
    int x, y;

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
            if (ev.xkey.keycode == 9)
                inborder = 0;
            break;
        case ButtonPress:
            if (ev.xbutton.button == 1) {
                XUngrabPointer(dpy, CurrentTime);
                resizemouse(NULL);
                return 0;
            }
            break;
        case MotionNotify:
            if ((ev.xmotion.time - lasttime) <=
                (1000 / (doubledraw ? 240 : 120)))
                continue;
            lasttime = ev.xmotion.time;

            getrootptr(&x, &y);

            if (y > c->y + c->h) // bottom
                c->h = y - c->y;

            if (y < c->y) { // top
                c->h = c->y + c->h - y;
                c->y = y;
            }

            if (x > c->x + c->w) // right
                c->w = x - c->x;

            if (x < c->x) { // left
                c->w = c->x + c->w - x;
                c->x = x;
            }

            if (c->w < 50)
                c->w = 50;
            if (c->h < 50)
                c->h = 50;

            resize(c, c->x, c->y, c->w, c->h, 1);
            break;
        }
    } while (ev.type != ButtonRelease && inborder);

    XUngrabPointer(dpy, CurrentTime);
    return 0;
}

void dragmouse(const Arg *arg) {
    int x, y, starty, startx, dragging, tabdragging, isactive, sinit;
    starty = 100;
    sinit = 0;
    dragging = 0;
    tabdragging = 0;
    XEvent ev;
    Time lasttime = 0;

    if (selmon->sel && selmon->sel->isfloating) {
        movemouse(NULL);
        return;
    }

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
            if ((ev.xmotion.time - lasttime) <= (1000 / 60))
                continue;
            lasttime = ev.xmotion.time;
            if (!sinit) {
                startx = ev.xmotion.x_root;
                starty = ev.xmotion.y_root;
                sinit = 1;
            }

            if (abs(ev.xmotion.x_root - startx) > (selmon->mw / 4) + 10) {
                if (!dragging) {
                    Arg a = {.i = (ev.xmotion.x_root < startx) ? -1 : 1};
                    viewtoleft(&a);
                    dragging = 1;
                }
            } else if (abs(ev.xmotion.y_root - starty) > 200) {
                if (!tabdragging) {
                    if (ev.xmotion.y_root > starty) {
                        togglescratchpad(NULL);
                    } else {
                        // overtoggle(NULL);
                    }
                    tabdragging = 1;
                }
            }

            break;
        }
    } while (ev.type != ButtonRelease);
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
            if ((ev.xmotion.time - lasttime) <= (1000 / 60))
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
            } else if (abs(ev.xmotion.y_root - starty) > 200) {
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

void drawwindow(const Arg *arg) {

    char str[100];
    int i;
    char strout[100] = {0};
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

    if (!fp)
        return;

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
            memset(tmpstring, 0, sizeof(tmpstring));
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

        if (c->isfloating) {
            resize(c, x, y, width, height, 1);
        } else {
            togglefloating(NULL);
            resize(c, x, y, width, height, 1);
        }
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
                setoverlay(NULL);
            } else {
                createoverlay(NULL);
                selmon->gesture = 11;
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
                (1000 / (doubledraw ? 240 : 120)))
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
    if ((m = recttomon(c->x, c->y, c->w, c->h)) != selmon) {
        sendmon(c, m);
        unfocus(selmon->sel, 0);
        selmon = m;
        focus(NULL);
    }
}

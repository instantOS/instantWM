/* See LICENSE file for copyright and license details. */

#include "bar.h"
#include "globals.h"
#include "instantwm.h"
#include "push.h"
#include "systray.h"
#include "toggles.h"
#include "util.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* Bar drawing constants */
#define DETAIL_BAR_HEIGHT_NORMAL 4 /* Normal detail bar height */
#define DETAIL_BAR_HEIGHT_HOVER 8  /* Hover state detail bar height */
#define STARTMENU_ICON_SIZE 14     /* Size of the start menu logo icon */
#define STARTMENU_ICON_INNER 6     /* Inner square of start menu icon */

/* External declarations not covered by headers */
extern int (*xerrorxlib)(Display *, XErrorEvent *);
extern int commandoffsets[20];
extern int pausedraw;

/* Helper: Get the width of the layout symbol area */
int get_layout_symbol_width(Monitor *m) { return TEXTW(m->ltsymbol) * 1.5; }

void clickstatus(const Arg *arg) {
    int x;
    int y;
    int i;
    getrootptr(&x, &y);
    i = 0;
    while (1) {
        if (i > 19 || (commandoffsets[i] == -1) || (commandoffsets[i] == 0)) {
            break;
        }
        if (x - selmon->mx < commandoffsets[i]) {
            break;
        }
        i++;
    }
    fprintf(stderr, "\ncounter: %d, x: %d, offset: %d", i, x - selmon->mx,
            commandoffsets[i]);
}

int drawstatusbar(Monitor *m, int bh, char *stext) {
    int ret;
    int i;
    int w;
    int x;
    int cmdcounter;
    short isCode = 0;
    char text_buf[1024];
    char *text = text_buf;
    char *p;

    strncpy(text, stext, sizeof(text_buf));
    text[sizeof(text_buf) - 1] = '\0';
    p = text;

    /* compute width of the status text */
    w = 0;
    i = -1;
    while (text[++i]) {
        if (text[i] == '^') {
            if (!isCode) {
                isCode = 1;
                text[i] = '\0';
                w += TEXTW(text) - lrpad;
                text[i] = '^';
                if (text[++i] == 'f') {
                    w += atoi(text + ++i);
                }
            } else {
                isCode = 0;
                text = text + i + 1;
                i = -1;
            }
        }
    }
    if (!isCode) {
        w += TEXTW(text) - lrpad;
    } else {
        isCode = 0;
    }
    text = p;
    statuswidth = w;
    w += 2; /* 1px padding on both sides */
    ret = x = m->ww - w - getsystraywidth();

    drw_setscheme(drw, statusscheme);
    drw_rect(drw, x, 0, w, bh, 1, 1);
    x++;

    /* process status text */
    i = -1;
    cmdcounter = 0;

    int customcolor = 0;

    while (text[++i]) {
        if (text[i] == '^' && !isCode) {
            isCode = 1;

            text[i] = '\0';
            w = TEXTW(text) - lrpad;
            drw_text(drw, x, 0, w, bh, 0, text, 0, 0);

            x += w;

            /* process code */
            while (text[++i] != '^') {
                if (text[i] == 'c') {
                    char buf[8];
                    memcpy(buf, text + i + 1, 7);
                    buf[7] = '\0';
                    customcolor = 1;
                    drw_clr_create(drw, &drw->scheme[ColBg], buf);
                    i += 7;
                } else if (text[i] == 't') {
                    char buf[8];
                    memcpy(buf, text + i + 1, 7);
                    buf[7] = '\0';
                    customcolor = 1;
                    drw_clr_create(drw, &drw->scheme[ColFg], buf);
                    i += 7;
                } else if (text[i] == 'd') {
                    drw_clr_create(drw, &drw->scheme[ColBg],
                                   statusbarcolors[ColBg]);
                    drw_clr_create(drw, &drw->scheme[ColFg],
                                   statusbarcolors[ColFg]);
                } else if (text[i] == 'r') {
                    int rx = atoi(text + ++i);
                    while (text[++i] != ',') {
                        ;
                    }
                    int ry = atoi(text + ++i);
                    while (text[++i] != ',') {
                        ;
                    }
                    int rw = atoi(text + ++i);
                    while (text[++i] != ',') {
                        ;
                    }
                    int rh = atoi(text + ++i);

                    drw_rect(drw, rx + x, ry, rw, rh, 1, 0);
                } else if (text[i] == 'f') {
                    x += atoi(text + ++i);
                } else if (text[i] == 'o') {
                    if (cmdcounter <= 20) {
                        commandoffsets[cmdcounter] = x;
                        cmdcounter++;
                    }
                }
            }

            text = text + i + 1;
            i = -1;
            isCode = 0;
        }
    }

    if (customcolor) {
        drw_clr_create(drw, &drw->scheme[ColBg], statusbarcolors[ColBg]);
        drw_clr_create(drw, &drw->scheme[ColFg], statusbarcolors[ColFg]);
    }

    if (cmdcounter < 20) {
        if (cmdcounter == 0) {
            commandoffsets[0] = -1;
        } else {
            commandoffsets[cmdcounter + 1] = -1;
        }
    }

    cmdcounter = 0;
    while (1) {
        if (cmdcounter > 19 || (commandoffsets[cmdcounter] == -1) ||
            (commandoffsets[cmdcounter] == 0)) {
            break;
        }
        cmdcounter++;
    }

    if (!isCode) {
        w = TEXTW(text) - lrpad;
        drw_text(drw, x, 0, w, bh, 0, text, 0, 0);
    }

    drw_setscheme(drw, statusscheme);

    return ret;
}

/* Helper: Draw the start menu icon (instantOS logo) */
static void draw_startmenu_icon(void) {
    int iconoffset = (bh - CLOSE_BUTTON_WIDTH) / 2;
    int startmenuinvert = (selmon->gesture == GestureStartMenu);

    if (tagprefix) {
        drw_setscheme(drw, tagscheme[SchemeNoHover][SchemeTagFocus]);
    } else {
        drw_setscheme(drw, statusscheme);
    }

    drw_rect(drw, 0, 0, startmenusize, bh, 1, startmenuinvert ? 0 : 1);
    drw_rect(drw, 5, iconoffset, STARTMENU_ICON_SIZE, STARTMENU_ICON_SIZE, 1,
             startmenuinvert ? 1 : 0);
    drw_rect(drw, 9, iconoffset + 4, STARTMENU_ICON_INNER, STARTMENU_ICON_INNER,
             1, startmenuinvert ? 0 : 1);
    drw_rect(drw, 19, iconoffset + STARTMENU_ICON_SIZE, STARTMENU_ICON_INNER,
             STARTMENU_ICON_INNER, 1, startmenuinvert ? 1 : 0);
}

/* Helper: Get the color scheme for a tag based on its state */
static Clr *get_tag_scheme(Monitor *m, unsigned int i,
                           unsigned int occupied_tags, int ishover) {
    if (occupied_tags & 1 << i) {
        /* Tag has clients */
        if (m == selmon && selmon->sel && selmon->sel->tags & 1 << i) {
            return tagscheme[ishover][SchemeTagFocus];
        }
        if (m->tagset[m->seltags] & 1 << i) {
            return tagscheme[ishover][SchemeTagNoFocus];
        }
        if (!selmon->showtags) {
            return tagscheme[ishover][SchemeTagFilled];
        }
        return tagscheme[ishover][SchemeTagInactive];
    }
    /* Tag is empty */
    if (m->tagset[m->seltags] & 1 << i) {
        return tagscheme[ishover][SchemeTagEmpty];
    }
    return tagscheme[ishover][SchemeTagInactive];
}

/* Helper: Draw all tag indicators and return the x position after them */
static int draw_tag_indicators(Monitor *m, int x, unsigned int occupied_tags,
                               unsigned int urg) {
    int w;
    int detail_height;
    int ishover;

    for (unsigned int i = 0; i < numtags; i++) {
        ishover = i == selmon->gesture - 1 ? SchemeHover : SchemeNoHover;
        if (i >= 9) {
            continue;
        }
        if (i == 8 && selmon->pertag->current_tag > 9) {
            i = selmon->pertag->current_tag - 1;
        }

        /* Do not draw vacant tags */
        if (selmon->showtags) {
            if (!(occupied_tags & 1 << i || m->tagset[m->seltags] & 1 << i)) {
                continue;
            }
        }

        w = TEXTW(tags[i]);
        drw_setscheme(drw, get_tag_scheme(m, i, occupied_tags, ishover));

        if (i == selmon->gesture - 1) {
            detail_height = DETAIL_BAR_HEIGHT_HOVER;
            if (bar_dragging) {
                drw_setscheme(drw, tagscheme[SchemeHover][SchemeTagFilled]);
            }
            drw_text(drw, x, 0, w, bh, lrpad / 2,
                     (showalttag ? tagsalt[i] : tags[i]), urg & 1 << i,
                     detail_height);
        } else {
            drw_text(drw, x, 0, w, bh, lrpad / 2,
                     (showalttag ? tagsalt[i] : tags[i]), urg & 1 << i,
                     DETAIL_BAR_HEIGHT_NORMAL);
        }
        x += w;
    }
    return x;
}

/* Helper: Draw layout indicator and return the x position after it */
static int draw_layout_indicator(Monitor *m, int x) {
    int w = get_layout_symbol_width(m);
    drw_setscheme(drw, statusscheme);
    return drw_text(drw, x, 0, w, bh, (w - TEXTW(m->ltsymbol)) * 0.5 + 10,
                    m->ltsymbol, 0, 0);
}

/* Helper: Get the color scheme for a window title based on its state */
static Clr *get_window_scheme(Client *c, int ishover) {
    if (c->mon->sel == c) {
        if (c == selmon->overlay) {
            return windowscheme[ishover][SchemeWinOverlayFocus];
        }
        if (c->issticky) {
            return windowscheme[ishover][SchemeWinStickyFocus];
        }
        return windowscheme[ishover][SchemeWinFocus];
    }
    if (c == selmon->overlay) {
        return windowscheme[ishover][SchemeWinOverlay];
    }
    if (c->issticky) {
        return windowscheme[ishover][SchemeWinSticky];
    }
    if (HIDDEN(c)) {
        return windowscheme[ishover][SchemeWinMinimized];
    }
    return windowscheme[ishover][SchemeWinNormal];
}

/* Helper: Draw the close button for the selected window */
static void draw_close_button(Client *c, int x) {
    int ishover =
        selmon->gesture != GestureCloseButton ? SchemeNoHover : SchemeHover;

    if (c->islocked) {
        drw_setscheme(drw, closebuttonscheme[ishover][SchemeCloseLocked]);
    } else if (c == selmon->fullscreen) {
        drw_setscheme(drw, closebuttonscheme[ishover][SchemeCloseFullscreen]);
    } else {
        drw_setscheme(drw, closebuttonscheme[ishover][SchemeCloseNormal]);
    }

    XSetForeground(drw->dpy, drw->gc, drw->scheme[ColBg].pixel);
    XFillRectangle(drw->dpy, drw->drawable, drw->gc, x + bh / 6,
                   (bh - CLOSE_BUTTON_WIDTH) / 2 -
                       !ishover * CLOSE_BUTTON_DETAIL,
                   CLOSE_BUTTON_WIDTH, CLOSE_BUTTON_HEIGHT);
    XSetForeground(drw->dpy, drw->gc, drw->scheme[ColDetail].pixel);
    XFillRectangle(drw->dpy, drw->drawable, drw->gc, x + bh / 6,
                   (bh - CLOSE_BUTTON_WIDTH) / 2 + CLOSE_BUTTON_HEIGHT -
                       !ishover * CLOSE_BUTTON_DETAIL,
                   CLOSE_BUTTON_WIDTH,
                   CLOSE_BUTTON_DETAIL + !ishover * CLOSE_BUTTON_DETAIL);
}

/* Helper: Draw a single window title */
static void draw_window_title(Monitor *m, Client *c, int x, int width) {
    int ishover =
        selmon->hoverclient && !selmon->gesture && c == selmon->hoverclient
            ? SchemeHover
            : SchemeNoHover;

    drw_setscheme(drw, get_window_scheme(c, ishover));

    /* Don't center text if it is too long */
    if (TEXTW(c->name) < width - 64) {
        drw_text(drw, x, 0, width, bh, (width - TEXTW(c->name)) * 0.5, c->name,
                 0, 4);
    } else {
        drw_text(drw, x, 0, width, bh, lrpad / 2 + 20, c->name, 0, 4);
    }

    if (m->sel == c) {
        draw_close_button(c, x);
        m->activeoffset = selmon->mx + x;
    }
}

/* Helper: Draw all window titles in the bar */
static void draw_window_titles(Monitor *m, int x, int w, int n) {
    if (n > 0) {
        int total_width = w + 1;
        int each_width = total_width / n;
        int remainder = total_width % n;

        for (Client *c = m->clients; c; c = c->next) {
            if (!ISVISIBLE(c)) {
                continue;
            }

            int this_width = each_width;
            if (remainder > 0) {
                this_width++;
                remainder--;
            }

            draw_window_title(m, c, x, this_width);
            x += this_width;
        }
    } else {
        drw_setscheme(drw, statusscheme);
        drw_rect(drw, x, 0, w, bh, 1, 1);
        drw_text(drw, x, 0, bh, bh, lrpad / 2, "", 0, 0);

        /* Display help message if no application is opened */
        if (!selmon->clients) {
            int titlewidth = TEXTW("Press space to launch an application") <
                                     m->bar_clients_width
                                 ? TEXTW("Press space to launch an application")
                                 : (m->bar_clients_width - bh);
            drw_text(drw,
                     x + bh +
                         ((m->bar_clients_width - bh) - titlewidth + 1) / 2,
                     0, titlewidth, bh, 0,
                     "Press space to launch an application", 0, 0);
        }
    }
}

void drawbar(Monitor *m) {
    if (pausedraw) {
        return;
    }

    int x;
    int w;
    int sw = 0;
    int n = 0;
    int stw = 0;
    unsigned int occupied_tags = 0;
    unsigned int urg = 0;
    Client *c;

    if (!m->showbar) {
        return;
    }

    if (showsystray && m == systraytomon(m)) {
        stw = getsystraywidth();
    }

    /* Draw status first so it can be overdrawn by tags later */
    if (m == selmon) {
        sw = m->ww - stw - drawstatusbar(m, bh, stext);
    }

    draw_startmenu_icon();
    resizebarwin(m);

    /* Collect client info for tags */
    for (c = m->clients; c; c = c->next) {
        if (ISVISIBLE(c)) {
            n++;
        }
        occupied_tags |= c->tags == 255 ? 0 : c->tags;
        if (c->isurgent) {
            urg |= c->tags;
        }
    }

    x = startmenusize;
    x = draw_tag_indicators(m, x, occupied_tags, urg);
    x = draw_layout_indicator(m, x);

    if ((w = m->ww - sw - x - stw) > bh) {
        draw_window_titles(m, x, w, n);
    }

    drw_setscheme(drw, statusscheme);
    m->bt = n;
    m->bar_clients_width = w;
    drw_map(drw, m->barwin, 0, 0, m->ww, bh);
}

void drawbars(void) {
    Monitor *m;

    for (m = mons; m; m = m->next) {
        drawbar(m);
    }
}

void resetbar() {
    if (!selmon->hoverclient && !selmon->gesture) {
        return;
    }
    selmon->hoverclient = NULL;
    selmon->gesture = GestureNone;
    if (altcursor) {
        resetcursor();
    }
    drawbar(selmon);
}

void updatestatus(void) {
    char text[512];
    if (!gettextprop(root, XA_WM_NAME, text, sizeof(text))) {
        strcpy(stext, "instantwm-" VERSION);
    } else {
        if (strncmp(text, "ipc:", 4) == 0) {
            return;
        }
        strncpy(stext, text, sizeof(stext) - 1);
        stext[sizeof(stext) - 1] = '\0';
    }
    drawbar(selmon);
    updatesystray();
}

void updatebarpos(Monitor *m) {
    m->wy = m->my;
    m->wh = m->mh;
    if (m->showbar) {
        m->wh -= bh;
        m->by = m->topbar ? m->wy : m->wy + m->wh;
        m->wy = m->topbar ? m->wy + bh : m->wy;
    } else {
        m->by = -bh;
    }
}

void resizebarwin(Monitor *m) {
    unsigned int w = m->ww;
    if (showsystray && m == systraytomon(m)) {
        w -= getsystraywidth();
    }
    XMoveResizeWindow(dpy, m->barwin, m->wx, m->by, w, bh);
}

void updatebars(void) {
    unsigned int w;
    Monitor *m;
    XSetWindowAttributes wa = {.override_redirect = True,
                               .background_pixmap = ParentRelative,
                               .event_mask = ButtonPressMask | ExposureMask |
                                             LeaveWindowMask};
    XClassHint ch = {"dwm", "dwm"};
    for (m = mons; m; m = m->next) {
        if (m->barwin) {
            continue;
        }
        w = m->ww;
        if (showsystray && m == systraytomon(m)) {
            w -= getsystraywidth();
        }
        m->barwin = XCreateWindow(
            dpy, root, m->wx, m->by, w, bh, 0, DefaultDepth(dpy, screen),
            CopyFromParent, DefaultVisual(dpy, screen),
            CWOverrideRedirect | CWBackPixmap | CWEventMask, &wa);
        // XDefineCursor(dpy, m->barwin, cursor[CurNormal]->cursor);
        if (showsystray && m == systraytomon(m)) {
            XMapRaised(dpy, systray->win);
        }
        XMapRaised(dpy, m->barwin);
        XSetClassHint(dpy, m->barwin, &ch);
    }
}

void togglebar(const Arg *arg) {
    int tmpnoanim;
    if (animated && clientcount() > 6) {
        animated = 0;
        tmpnoanim = 1;
    } else {
        tmpnoanim = 0;
    }

    selmon->showbar = selmon->pertag->showbars[selmon->pertag->current_tag] =
        !selmon->showbar;
    updatebarpos(selmon);
    resizebarwin(selmon);
    if (showsystray) {
        XWindowChanges wc;
        if (!selmon->showbar) {
            wc.y = -bh;
        } else {
            wc.y = 0;
            if (!selmon->topbar) {
                wc.y = selmon->mh - bh;
            }
        }
        XConfigureWindow(dpy, systray->win, CWY, &wc);
    }
    arrange(selmon);
    if (tmpnoanim) {
        animated = 1;
    }
    if (selmon->overlaystatus) {
        tmpnoanim = animated;
        animated = 0;
        selmon->overlaystatus = 0;
        showoverlay(NULL);
        animated = tmpnoanim;
    }
}

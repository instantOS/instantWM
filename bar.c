/* See LICENSE file for copyright and license details. */

#include "bar.h"
#include "systray.h"
#include "toggles.h"
#include "util.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* extern declarations for variables from instantwm.c and config.h */
extern Display *dpy;
extern Drw *drw;
extern Monitor *selmon;
extern Monitor *mons;
extern Window root;
extern char stext[1024];
extern int bh;
extern int lrpad;
extern int (*xerrorxlib)(Display *, XErrorEvent *);
extern int commandoffsets[20];
extern int bardragging;
extern int showalttag;
extern int tagprefix;
extern int altcursor;
extern int statuswidth; /* from instantwm.c */
extern int pausedraw;

/* Schemes */
extern Clr ***tagscheme;
extern Clr *statusscheme;
extern Clr ***windowscheme;
extern Clr ***closebuttonscheme;

/* config.h values */
extern const int showsystray;
extern const unsigned int systrayspacing;
extern const char *tagsalt[];
extern char tags[][16]; /* MAX_TAGLEN is 16 in config.h */
extern const unsigned int startmenusize;
extern const char *statusbarcolors[];

extern int get_blw(Monitor *m);

void clickstatus(const Arg *arg) {
    int x, y, i;
    getrootptr(&x, &y);
    i = 0;
    while (1) {
        if (i > 19 || (commandoffsets[i] == -1) || (commandoffsets[i] == 0))
            break;
        if (x - selmon->mx < commandoffsets[i])
            break;
        i++;
    }
    fprintf(stderr, "\ncounter: %d, x: %d, offset: %d", i, x - selmon->mx,
            commandoffsets[i]);
}

int drawstatusbar(Monitor *m, int bh, char *stext) {
    int ret, i, w, x, len, cmdcounter;
    short isCode = 0;
    char *text;
    char *p;

    len = strlen(stext) + 1;
    if (!(text = (char *)malloc(sizeof(char) * len)))
        die("malloc");
    p = text;
    memcpy(text, stext, len);

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
                if (text[++i] == 'f')
                    w += atoi(text + ++i);
            } else {
                isCode = 0;
                text = text + i + 1;
                i = -1;
            }
        }
    }
    if (!isCode)
        w += TEXTW(text) - lrpad;
    else
        isCode = 0;
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
                    memcpy(buf, (char *)text + i + 1, 7);
                    buf[7] = '\0';
                    customcolor = 1;
                    drw_clr_create(drw, &drw->scheme[ColBg], buf);
                    i += 7;
                } else if (text[i] == 't') {
                    char buf[8];
                    memcpy(buf, (char *)text + i + 1, 7);
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
                    while (text[++i] != ',')
                        ;
                    int ry = atoi(text + ++i);
                    while (text[++i] != ',')
                        ;
                    int rw = atoi(text + ++i);
                    while (text[++i] != ',')
                        ;
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
        if (cmdcounter == 0)
            commandoffsets[0] = -1;
        else
            commandoffsets[cmdcounter + 1] = -1;
    }

    cmdcounter = 0;
    while (1) {
        if (cmdcounter > 19 || (commandoffsets[cmdcounter] == -1) ||
            (commandoffsets[cmdcounter] == 0))
            break;
        cmdcounter++;
    }

    if (!isCode) {
        w = TEXTW(text) - lrpad;
        drw_text(drw, x, 0, w, bh, 0, text, 0, 0);
    }

    drw_setscheme(drw, statusscheme);
    free(p);

    return ret;
}

void drawbar(Monitor *m) {
    if (pausedraw)
        return;

    int x, w, sw = 0, n = 0, stw = 0, roundw, iconoffset, ishover;

    unsigned int i, occ = 0, urg = 0;
    Client *c;

    if (!m->showbar)
        return;

    if (showsystray && m == systraytomon(m))
        stw = getsystraywidth();

    /* draw status first so it can be overdrawn by tags later */
    if (m == selmon) { /* status is only drawn on selected monitor */
        sw = m->ww - stw - drawstatusbar(m, bh, stext);
    }

    // draw start menu icon with instantOS logo
    if (tagprefix)
        drw_setscheme(drw, tagscheme[SchemeNoHover][SchemeTagFocus]);
    else
        drw_setscheme(drw, statusscheme);

    iconoffset = (bh - 20) / 2;
    int startmenuinvert = (selmon->gesture == 13);
    drw_rect(drw, 0, 0, startmenusize, bh, 1, startmenuinvert ? 0 : 1);
    drw_rect(drw, 5, iconoffset, 14, 14, 1, startmenuinvert ? 1 : 0);
    drw_rect(drw, 9, iconoffset + 4, 6, 6, 1, startmenuinvert ? 0 : 1);
    drw_rect(drw, 19, iconoffset + 14, 6, 6, 1, startmenuinvert ? 1 : 0);

    resizebarwin(m);

    // check for clients on tag
    for (c = m->clients; c; c = c->next) {
        if (ISVISIBLE(c))
            n++;
        occ |= c->tags == 255 ? 0 : c->tags;

        if (c->isurgent)
            urg |= c->tags;
    }

    x = startmenusize;

    // render all tag indicators
    for (i = 0; i < 21; i++) { /* LENGTH(tags) is 21 */
        ishover = i == selmon->gesture - 1 ? SchemeHover : SchemeNoHover;
        if (i >= 9)
            continue;
        if (i == 8 && selmon->pertag->curtag > 9)
            i = selmon->pertag->curtag - 1;

        /* do not draw vacant tags */
        if (selmon->showtags) {
            if (!(occ & 1 << i || m->tagset[m->seltags] & 1 << i))
                continue;
        }

        w = TEXTW(tags[i]);

        // tag has client
        if (occ & 1 << i) {
            if (m == selmon && selmon->sel && selmon->sel->tags & 1 << i) {
                drw_setscheme(drw, tagscheme[ishover][SchemeTagFocus]);
            } else {
                // tag is active, has clients but is not in focus
                if (m->tagset[m->seltags] & 1 << i) {
                    drw_setscheme(drw, tagscheme[ishover][SchemeTagNoFocus]);
                } else {
                    // do not color tags if vacant tags are hidden
                    if (!selmon->showtags) {
                        drw_setscheme(drw, tagscheme[ishover][SchemeTagFilled]);
                    } else {
                        drw_setscheme(drw,
                                      tagscheme[ishover][SchemeTagInactive]);
                    }
                }
            }
        } else { // tag does not have a client
            if (m->tagset[m->seltags] & 1 << i) {
                drw_setscheme(drw, tagscheme[ishover][SchemeTagEmpty]);
            } else {
                drw_setscheme(drw, tagscheme[ishover][SchemeTagInactive]);
            }
        }

        if (i == selmon->gesture - 1) {
            roundw = 8;
            if (bardragging) {
                drw_setscheme(drw, tagscheme[SchemeHover][SchemeTagFilled]);
            }
            drw_text(drw, x, 0, w, bh, lrpad / 2,
                     (showalttag ? tagsalt[i] : tags[i]), urg & 1 << i, roundw);

        } else {
            drw_text(drw, x, 0, w, bh, lrpad / 2,
                     (showalttag ? tagsalt[i] : tags[i]), urg & 1 << i, 4);
        }
        x += w;
    }

    // render layout indicator
    w = get_blw(m);
    drw_setscheme(drw, statusscheme);
    x = drw_text(drw, x, 0, w, bh, (w - TEXTW(m->ltsymbol)) * 0.5 + 10,
                 m->ltsymbol, 0, 0);

    if ((w = m->ww - sw - x - stw) > bh) {
        if (n > 0) {
            // Calculate the base width and remainder before the loop
            int total_width = w + 1; // Total available width for titles
            int each_width = total_width / n; // Base width for each title
            int remainder =
                total_width % n; // Remainder to distribute extra pixels

            // render all window titles
            for (c = m->clients; c; c = c->next) {
                if (!ISVISIBLE(c))
                    continue;

                int this_width = each_width;

                if (remainder > 0) {
                    this_width++; // Add one pixel to account for the remainder
                    remainder--;
                }

                ishover = selmon->hoverclient && !selmon->gesture &&
                                  c == selmon->hoverclient
                              ? SchemeHover
                              : SchemeNoHover;

                if (m->sel == c) {
                    if (c == selmon->overlay) {
                        drw_setscheme(
                            drw, windowscheme[ishover][SchemeWinOverlayFocus]);
                    } else if (c->issticky) {
                        drw_setscheme(
                            drw, windowscheme[ishover][SchemeWinStickyFocus]);
                    } else {
                        drw_setscheme(drw,
                                      windowscheme[ishover][SchemeWinFocus]);
                    }
                } else {
                    if (c == selmon->overlay) {
                        drw_setscheme(drw,
                                      windowscheme[ishover][SchemeWinOverlay]);
                    } else if (c->issticky) {
                        drw_setscheme(drw,
                                      windowscheme[ishover][SchemeWinSticky]);
                    } else if (HIDDEN(c)) {
                        drw_setscheme(
                            drw, windowscheme[ishover][SchemeWinMinimized]);
                    } else {
                        drw_setscheme(drw,
                                      windowscheme[ishover][SchemeWinNormal]);
                    }
                }

                // don't center text if it is too long
                if (TEXTW(c->name) < this_width - 64) {
                    drw_text(drw, x, 0, this_width, bh,
                             (this_width - TEXTW(c->name)) * 0.5, c->name, 0,
                             4);
                } else {
                    drw_text(drw, x, 0, this_width, bh, lrpad / 2 + 20, c->name,
                             0, 4);
                }

                if (m->sel == c) {
                    // render close button
                    ishover =
                        selmon->gesture != 12 ? SchemeNoHover : SchemeHover;

                    if (c->islocked) {
                        drw_setscheme(
                            drw, closebuttonscheme[ishover][SchemeCloseLocked]);
                    } else if (c == selmon->fullscreen) {
                        drw_setscheme(
                            drw,
                            closebuttonscheme[ishover][SchemeCloseFullscreen]);
                    } else {
                        drw_setscheme(
                            drw, closebuttonscheme[ishover][SchemeCloseNormal]);
                    }

                    XSetForeground(drw->dpy, drw->gc, drw->scheme[ColBg].pixel);
                    XFillRectangle(drw->dpy, drw->drawable, drw->gc, x + bh / 6,
                                   (bh - 20) / 2 - !ishover * 4, 20, 16);
                    XSetForeground(drw->dpy, drw->gc,
                                   drw->scheme[ColDetail].pixel);
                    XFillRectangle(drw->dpy, drw->drawable, drw->gc, x + bh / 6,
                                   (bh - 20) / 2 + 16 - !ishover * 4, 20,
                                   4 + !ishover * 4);

                    // save position of focussed window title on bar
                    m->activeoffset = selmon->mx + x;
                }
                x += this_width;
            }
        } else {
            drw_setscheme(drw, statusscheme);
            drw_rect(drw, x, 0, w, bh, 1, 1);
            // render shutdown button
            drw_text(drw, x, 0, bh, bh, lrpad / 2, "", 0, 0);
            // display help message if no application is opened
            if (!selmon->clients) {
                int titlewidth =
                    TEXTW("Press space to launch an application") < m->btw
                        ? TEXTW("Press space to launch an application")
                        : (m->btw - bh);
                drw_text(drw, x + bh + ((m->btw - bh) - titlewidth + 1) / 2, 0,
                         titlewidth, bh, 0,
                         "Press space to launch an application", 0, 0);
            }
        }
    }

    drw_setscheme(drw, statusscheme);

    m->bt = n;
    m->btw = w;
    drw_map(drw, m->barwin, 0, 0, m->ww, bh);
}

void drawbars(void) {
    Monitor *m;

    for (m = mons; m; m = m->next)
        drawbar(m);
}

void resetbar() {
    if (!selmon->hoverclient && !selmon->gesture)
        return;
    selmon->hoverclient = NULL;
    selmon->gesture = 0;
    if (altcursor)
        resetcursor();
    drawbar(selmon);
}

void updatestatus(void) {
    char text[512];
    if (!gettextprop(root, XA_WM_NAME, text, sizeof(text))) {
        strcpy(stext, "instantwm-" VERSION);
    } else {
        if (strncmp(text, "ipc:", 4) == 0)
            return;
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
    } else
        m->by = -bh;
}

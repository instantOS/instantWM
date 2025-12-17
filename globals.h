/* See LICENSE file for copyright and license details.
 *
 * Centralized extern declarations for commonly shared global variables.
 * Include this header instead of repeating extern declarations in each .c file.
 */

#ifndef GLOBALS_H
#define GLOBALS_H

#include "instantwm.h"

/* ========== Core X11 globals ========== */
extern Display *dpy;
extern Drw *drw;
extern Window root;

/* ========== Monitor state ========== */
extern Monitor *mons;    /* linked list of all monitors */
extern Monitor *selmon;  /* currently selected monitor */

/* ========== Bar dimensions ========== */
extern int bh;     /* bar height */
extern int lrpad;  /* left/right padding for text */

/* ========== Feature flags ========== */
extern int animated;               /* animation enabled */
extern int focusfollowsmouse;      /* focus follows mouse for tiled */
extern int focusfollowsfloatmouse; /* focus follows mouse for floating */
extern int altcursor;              /* current alternate cursor state */
extern int doubledraw;             /* high refresh rate mode */

/* ========== Bar state ========== */
extern int bar_dragging;  /* currently dragging on bar */
extern int tagwidth;      /* cached tag area width */
extern int statuswidth;   /* status text width */
extern int showalttag;    /* show alternate tag icons */
extern int tagprefix;     /* tag prefix mode active */
extern char stext[];      /* status text buffer */

/* ========== Atoms ========== */
extern Atom wmatom[];
extern Atom netatom[];
extern Atom xatom[];
extern Atom motifatom;

/* ========== Cursors ========== */
extern Cur *cursor[];     /* cursor array indexed by Cur enum */

/* ========== Color schemes ========== */
extern Clr *borderscheme;
extern Clr *statusscheme;
extern Clr ***tagscheme;
extern Clr ***windowscheme;
extern Clr ***closebuttonscheme;

/* ========== Systray (from config.h) ========== */
extern const int showsystray;
extern const unsigned int systraypinning;
extern const unsigned int systrayspacing;
extern Systray *systray;

/* ========== Config values ========== */
extern const unsigned int startmenusize;
extern const unsigned int snap;
extern const int resizehints;
extern char tags[][16];
extern const char *tagsalt[];
extern const Layout layouts[];
extern int numtags;

#endif /* GLOBALS_H */

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
extern int screen;
extern int sw, sh; /* X display screen geometry width, height */

/* ========== Monitor state ========== */
extern Monitor *mons;   /* linked list of all monitors */
extern Monitor *selmon; /* currently selected monitor */

/* ========== Bar dimensions ========== */
extern int bh;    /* bar height */
extern int lrpad; /* left/right padding for text */

/* ========== Feature flags ========== */
extern int animated;               /* animation enabled */
extern int focusfollowsmouse;      /* focus follows mouse for tiled */
extern int focusfollowsfloatmouse; /* focus follows mouse for floating */
extern int altcursor;              /* current alternate cursor state */
extern int doubledraw;             /* high refresh rate mode */
extern int specialnext;

/* ========== Bar state ========== */
extern int bar_dragging; /* currently dragging on bar */
extern int tagwidth;     /* cached tag area width */
extern int statuswidth;  /* status text width */
extern int showalttag;   /* show alternate tag icons */
extern int tagprefix;    /* tag prefix mode active */
extern char stext[1024]; /* status text buffer */

/* ========== Atoms ========== */
extern Atom wmatom[];
extern Atom netatom[];
extern Atom xatom[];
extern Atom motifatom;

/* ========== Cursors ========== */
extern Cur *cursor[]; /* cursor array indexed by Cur enum */

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
extern size_t keys_len;
extern size_t dkeys_len;
extern size_t commands_len;
extern size_t buttons_len;
extern size_t layouts_len;
extern size_t rules_len;
extern size_t fonts_len;

/* ========== Variables from config.c (non-static in config.h) ========== */
extern Xcommand commands[];
extern Button buttons[];
extern const char *fonts[];
extern const char *tagcolors[2][5][3];
extern const char *windowcolors[2][7][3];
extern const char *closebuttoncolors[2][3][3];
extern const char *bordercolors[];
extern const char *statusbarcolors[];
extern Key keys[];
extern Key dkeys[];
extern Rule rules[];
extern ResourcePref resources[];

/* Config variables needed in other files */
extern unsigned int tagmask;
extern unsigned int borderpx;
extern int decorhints;
extern float mfact;
extern int nmaster;
extern int showbar;
extern int topbar;
extern int barheight;
extern char xresourcesfont[30];
extern char instantmenumon[2];
extern const char *instantmenucmd[];
extern const char *instantshutdowncmd[];
extern const char *startmenucmd[];

#endif /* GLOBALS_H */

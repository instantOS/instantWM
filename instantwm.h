#ifndef INSTANTWM_H
#define INSTANTWM_H

#include <X11/cursorfont.h>
#include <X11/keysym.h>
#include <X11/Xatom.h>
#include <X11/Xft/Xft.h>
#include <X11/Xlib.h>
#include <X11/Xproto.h>
#include <X11/Xresource.h>
#include <X11/Xutil.h>

#ifdef XINERAMA
#include <X11/extensions/Xinerama.h>
#endif /* XINERAMA */

#include "drw.h"


/* macros */
#define BUTTONMASK              (ButtonPressMask|ButtonReleaseMask)
#define CLEANMASK(mask)         (mask & ~(numlockmask|LockMask) & (ShiftMask|ControlMask|Mod1Mask|Mod2Mask|Mod3Mask|Mod4Mask|Mod5Mask))
#define INTERSECT(x,y,w,h,m)    (MAX(0, MIN((x)+(w),(m)->wx+(m)->ww) - MAX((x),(m)->wx)) \
                               * MAX(0, MIN((y)+(h),(m)->wy+(m)->wh) - MAX((y),(m)->wy)))
#define ISVISIBLE(C)            ((C->tags & C->mon->tagset[C->mon->seltags]) || C->issticky)
#define HIDDEN(C)               ((getstate(C->win) == IconicState))
#define LENGTH(X)               (sizeof X / sizeof X[0])
#define MOUSEMASK               (BUTTONMASK|PointerMotionMask)
#define WIDTH(X)                ((X)->w + 2 * (X)->bw)
#define HEIGHT(X)               ((X)->h + 2 * (X)->bw)
#define TAGMASK                 ((1 << LENGTH(tags)) - 1)
#define TEXTW(X)                (drw_fontset_getwidth(drw, (X)) + lrpad)

#define MWM_HINTS_FLAGS_FIELD       0
#define MWM_HINTS_DECORATIONS_FIELD 2
#define MWM_HINTS_DECORATIONS       (1 << 1)
#define MWM_DECOR_ALL               (1 << 0)
#define MWM_DECOR_BORDER            (1 << 1)
#define MWM_DECOR_TITLE             (1 << 3)

/* XEMBED messages */
#define XEMBED_EMBEDDED_NOTIFY      0
#define XEMBED_WINDOW_ACTIVATE      1
#define XEMBED_FOCUS_IN             4
#define XEMBED_MODALITY_ON         10

#define XEMBED_MAPPED              (1 << 0)
#define XEMBED_WINDOW_ACTIVATE      1
#define XEMBED_WINDOW_DEACTIVATE    2

#define VERSION_MAJOR               0
#define VERSION_MINOR               0
#define XEMBED_EMBEDDED_VERSION (VERSION_MAJOR << 16) | VERSION_MINOR
#define SYSTEM_TRAY_REQUEST_DOCK    0

/* enums */
enum { CurNormal, CurResize, CurMove, CurClick, CurHor, CurVert, CurTL, CurTR, CurBL, CurBR, CurLast }; /* cursor */
/* enum { SchemeNorm, SchemeSel, SchemeHid, SchemeTags, SchemeActive, SchemeAddActive, SchemeEmpty, SchemeHover, SchemeClose, SchemeHoverTags }; /1* color schemes *1/ */
enum { NetSupported, NetWMName, NetWMState, NetWMCheck,
       NetSystemTray, NetSystemTrayOP, NetSystemTrayOrientation, NetSystemTrayOrientationHorz,
       NetWMFullscreen, NetActiveWindow, NetWMWindowType,
       NetWMWindowTypeDialog, NetClientList, NetClientInfo, NetLast }; /* EWMH atoms */
enum { Manager, Xembed, XembedInfo, XLast }; /* Xembed atoms */
enum { WMProtocols, WMDelete, WMState, WMTakeFocus, WMLast }; /* default atoms */
enum { ClkTagBar, ClkLtSymbol, ClkStatusText, ClkWinTitle,
       ClkClientWin, ClkRootWin, ClkCloseButton, ClkShutDown, ClkSideBar, ClkStartMenu, ClkLast }; /* clicks */



////// Colorscheme enums ////// 
// each element has the possibility of a hover over
enum { SchemeHover, SchemeNoHover, SchemeHoverLast };
// tag states
enum { SchemeTagInactive, SchemeTagFilled, SchemeTagFocus, SchemeTagNoFocus, SchemeTagEmpty, SchemeTagLast };
// window states
enum { SchemeWinFocus, SchemeWinMinimized, SchemeWinNormal,
    SchemeWinStickyFocus, SchemeWinSticky, SchemeWinOverlay, SchemeWinOverlayFocus };
// close button
enum { SchemeCloseNormal, SchemeCloseLocked, SchemeCloseFullscreen, SchemeCloseLast };
// window border states
enum { SchemeBorderNormal, SchemeBorderFloatFocus, SchemeBorderTileFocus, SchemeBorderSnap, SchemeBorderLast };

typedef union {
	int i;
	unsigned int ui;
	float f;
	const void *v;
} Arg;

typedef struct {
	unsigned int click;
	unsigned int mask;
	unsigned int button;
	void (*func)(const Arg *arg);
	const Arg arg;
} Button;

typedef struct Monitor Monitor;
typedef struct Client Client;
struct Client {
	char name[256];
	float mina, maxa;
	int x, y, w, h;
	int sfx, sfy, sfw, sfh; /* stored float geometry, used on mode revert */
	int oldx, oldy, oldw, oldh;
	int basew, baseh, incw, inch, maxw, maxh, minw, minh, hintsvalid;
	int bw, oldbw;
	unsigned int tags;
	int isfixed, isfloating, isurgent, neverfocus, oldstate, isfullscreen, isfakefullscreen, islocked, issticky, snapstatus;
	Client *next;
	Client *snext;
	Monitor *mon;
	Window win;
};

typedef struct {
	unsigned int mod;
	KeySym keysym;
	void (*func)(const Arg *);
	const Arg arg;
} Key;


typedef struct {
    char *cmd;
	void (*func)(const Arg *);
	const Arg arg;
    unsigned int type;
} Xcommand;

typedef struct {
	const char *symbol;
	void (*arrange)(Monitor *);
} Layout;

typedef struct Pertag Pertag;
struct Monitor {
	char ltsymbol[16];
	float mfact;
	int nmaster;
	int num;
	int by;               /* bar geometry */
	int btw;              /* width of tasks portion of bar */
	int bt;               /* number of tasks */
	int mx, my, mw, mh;   /* screen size */
	int wx, wy, ww, wh;   /* window area  */
	unsigned int seltags;
	unsigned int sellt;
	unsigned int tagset[2];
	unsigned int activeoffset;
	unsigned int titleoffset;
	unsigned int clientcount;
	int showbar;
	int topbar;
	Client *clients;
	Client *sel;
	Client *overlay;
	Client *activescratchpad;
	Client *fullscreen;
	int overlaystatus;
    int overlaymode;
	int scratchvisible;
	int gesture;
	Client *stack;
	Client *hoverclient;
	Monitor *next;
	Window barwin;
	const Layout *lt[2];
	unsigned int showtags;
	Pertag *pertag;
};

typedef struct {
	const char *class;
	const char *instance;
	const char *title;
	unsigned int tags;
	int isfloating;
	int monitor;
} Rule;

/* Xresources preferences */
enum resource_type {
	STRING = 0,
	INTEGER = 1,
	FLOAT = 2
};

typedef struct {
	char *name;
	enum resource_type type;
	void *dst;
} ResourcePref;

typedef struct {
    char *name;
    int type;
} SchemePref;

typedef struct Systray   Systray;
struct Systray {
	Window win;
	Client *icons;
};

/* function declarations */
void applyrules(Client *c);
int applysizehints(Client *c, int *x, int *y, int *w, int *h, int interact);
void arrange(Monitor *m);
void arrangemon(Monitor *m);
void resetcursor();
void attach(Client *c);
void attachstack(Client *c);
void buttonpress(XEvent *e);
void checkotherwm(void);
void cleanup(void);
void cleanupmon(Monitor *mon);
void clientmessage(XEvent *e);
void configure(Client *c);
void configurenotify(XEvent *e);
void configurerequest(XEvent *e);
Monitor *createmon(void);
void cyclelayout(const Arg *arg);
void destroynotify(XEvent *e);
void detach(Client *c);
void detachstack(Client *c);
Monitor *dirtomon(int dir);
void drawbar(Monitor *m);
void drawbars(void);
int drawstatusbar(Monitor *m, int bh, char* text);
void enternotify(XEvent *e);
void expose(XEvent *e);
void focus(Client *c);
void focusin(XEvent *e);
void focusmon(const Arg *arg);
void focusnmon(const Arg *arg);
void followmon(const Arg *arg);
void focusstack(const Arg *arg);
void upkey(const Arg *arg);
void downkey(const Arg *arg);
void spacetoggle(const Arg *arg);
Atom getatomprop(Client *c, Atom prop);
int getrootptr(int *x, int *y);
long getstate(Window w);
unsigned int getsystraywidth();
int gettextprop(Window w, Atom atom, char *text, unsigned int size);
void grabbuttons(Client *c, int focused);
void grabkeys(void);
void hide(Client *c);
void incnmaster(const Arg *arg);
void keypress(XEvent *e);
int xcommand(void);
void killclient(const Arg *arg);
void manage(Window w, XWindowAttributes *wa);
void mappingnotify(XEvent *e);
void maprequest(XEvent *e);
void motionnotify(XEvent *e);
void movemouse(const Arg *arg);
void dragmouse(const Arg *arg);
void gesturemouse(const Arg *arg);
int resizeborder(const Arg *arg);
void dragrightmouse(const Arg *arg);
void drawwindow(const Arg *arg);
void dragtag(const Arg *arg);
void moveresize(const Arg *arg);
void distributeclients(const Arg *arg);
void keyresize(const Arg *arg);
void centerwindow();
void resetnametag(const Arg *arg);
void nametag(const Arg *arg);
Client *nexttiled(Client *c);
void pop(Client *);
void shutkill(const Arg *arg);
void propertynotify(XEvent *e);
void quit(const Arg *arg);
Monitor *recttomon(int x, int y, int w, int h);
void removesystrayicon(Client *i);
void resize(Client *c, int x, int y, int w, int h, int interact);
void applysize(Client *c);
void resetsticky(Client *c);
void applysnap(Client *c, Monitor *m);
int unhideone();
int allclientcount();
int clientcountmon(Monitor *m);
void resizebarwin(Monitor *m);
void resizeclient(Client *c, int x, int y, int w, int h);
void resizemouse(const Arg *arg);
void forceresizemouse(const Arg *arg);
void resizeaspectmouse(const Arg *arg);
void resizerequest(XEvent *e);
void restack(Monitor *m);
void animateclient(Client *c, int x, int y, int w, int h, int frames, int resetpos);
void checkanimate(Client *c, int x, int y, int w, int h, int frames, int resetpos);
void run(void);
void runAutostart(void);
void scan(void);
int sendevent(Window w, Atom proto, int m, long d0, long d1, long d2, long d3, long d4);
void sendmon(Client *c, Monitor *m);
int gettagwidth();
int getxtag(int ix);
void setclientstate(Client *c, long state);
void setclienttagprop(Client *c);
void setfocus(Client *c);
void setfullscreen(Client *c, int fullscreen);
void setlayout(const Arg *arg);
void commandlayout(const Arg *arg);
void commandprefix(const Arg *arg);
void setmfact(const Arg *arg);
void setup(void);
void seturgent(Client *c, int urg);
void show(Client *c);
void showhide(Client *c);
void sigchld(int unused);
void spawn(const Arg *arg);
void clickstatus(const Arg *arg);
Monitor *systraytomon(Monitor *m);
Client *getcursorclient();
void tag(const Arg *arg);
void tagall(const Arg *arg);
void followtag(const Arg *arg);
void swaptags(const Arg *arg);
void followview(const Arg *arg);
void tagmon(const Arg *arg);
void tagtoleft(const Arg *arg);
void tagtoright(const Arg *arg);
void uppress(const Arg *arg);
void downpress(const Arg *arg);
void togglealttag(const Arg *arg);
void alttabfree(const Arg *arg);
void toggleanimated(const Arg *arg);
void setborderwidth(const Arg *arg);
void togglefocusfollowsmouse(const Arg *arg);
void togglefocusfollowsfloatmouse(const Arg *arg);
void toggledoubledraw(const Arg *arg);
void togglefakefullscreen(const Arg *arg);
void togglelocked(const Arg *arg);
void toggleshowtags(const Arg *arg);
void togglebar(const Arg *arg);
void togglefloating(const Arg *arg);
void togglesticky(const Arg *arg);
void toggleprefix(const Arg *arg);
void toggletag(const Arg *arg);
void togglescratchpad(const Arg *arg);
void createscratchpad(const Arg *arg);
void toggleview(const Arg *arg);
void hidewin(const Arg *arg);
void redrawwin(const Arg *arg);
void unhideall(const Arg *arg);
void closewin(const Arg *arg);
void unfocus(Client *c, int setfocus);
void unmanage(Client *c, int destroyed);
void unmapnotify(XEvent *e);
void updatebarpos(Monitor *m);
void verifytagsxres(void);
void updatebars(void);
void updateclientlist(void);
int updategeom(void);
void updatemotifhints(Client *c);
void updatenumlockmask(void);
void updatesizehints(Client *c);
void updatestatus(void);
void updatesystray(void);
void updatesystrayicongeom(Client *i, int w, int h);
void updatesystrayiconstate(Client *i, XPropertyEvent *ev);
void updatetitle(Client *c);
void updatewindowtype(Client *c);
void updatewmhints(Client *c);
void view(const Arg *arg);
void warp(const Client *c);
void forcewarp(const Client *c);
void warpinto(const Client *c);
void warpfocus();
void viewtoleft(const Arg *arg);
void animleft(const Arg *arg);
void animright(const Arg *arg);
void moveleft(const Arg *arg);
void viewtoright(const Arg *arg);
void moveright(const Arg *arg);

void scaleclient(Client *c, int scale);
void upscaleclient(const Arg *arg);
void downscaleclient(const Arg *arg);

void overtoggle(const Arg *arg);
void lastview(const Arg *arg);
void fullovertoggle(const Arg *arg);

void setspecialnext(const Arg *arg);

void directionfocus(const Arg *arg);

Client *wintoclient(Window w);
Monitor *wintomon(Window w);
Client *wintosystrayicon(Window w);
void winview(const Arg* arg);

int xerror(Display *dpy, XErrorEvent *ee);
int xerrordummy(Display *dpy, XErrorEvent *ee);
int xerrorstart(Display *dpy, XErrorEvent *ee);
void zoom(const Arg *arg);
void load_xresources(void);
void resource_load(XrmDatabase db, char *name, enum resource_type rtype, void *dst);

void keyrelease(XEvent *e);
void setoverlay();
void desktopset();
void createdesktop();
void createoverlay();
void tempfullscreen();

void savefloating(Client *c);
void restorefloating(Client *c);

void savebw(Client *c);
void restorebw(Client *c);

void shiftview(const Arg *arg);
void focuslastclient(const Arg *arg);

void resetoverlay();
void showoverlay();
void hideoverlay();
void changefloating(Client *c);
void resetbar();

extern Monitor *selmon;
extern int bh;
extern int animated;
extern Display *dpy;

#endif

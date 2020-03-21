/* See LICENSE file for copyright and license details. */
#include <X11/XF86keysym.h>
/* appearance */
static const unsigned int borderpx = 3;		  /* border pixel of windows */
static const unsigned int snap = 32;		  /* snap pixel */
static const unsigned int systraypinning = 0; /* 0: sloppy systray follows selected monitor, >0: pin systray to monitor X */
static const unsigned int systrayspacing = 2; /* systray spacing */
static const int systraypinningfailfirst = 1; /* 1: if pinning fails, display systray on the first monitor, False: display systray on the last monitor*/
static const int showsystray = 1;			  /* 0 means no systray */
static const int showbar = 1;				  /* 0 means no bar */
static const int topbar = 1;				  /* 0 means bottom bar */
static const char *fonts[] = {"Cantarell-Regular:size=12", "Fira Code Nerd Font:size=12"};
static const char col_gray1[] = "#292f3a"; /* top bar d */
static const char col_gray2[] = "#ffffff";/*unfocused fonts d */
static const char col_gray3[] = "#747c90";/*unfocused border d */
static const char col_gray4[] = "#ffffff";/*focused fonts d */
static const char col_gray5[] = "#4dadd4";/*focused windows d */
static const char col_cyan[] = "#5294E2";/*focused instantmenu or topbar d */
static const char col_border1[] = "#73d216";/*focused instantmenu or topbar d */

static const char *colors[][4] = {
	/*               fg         bg         border   	float*/
	[SchemeNorm] = { col_gray2, col_gray1, col_gray3, col_border1 },
	[SchemeSel]  = { col_gray4, col_cyan,  col_gray5, col_border1  },
	[SchemeHid]  = { col_cyan,  col_gray1, col_cyan, col_border1  },
};

/* tagging */
static const char *tags[] = { "1", "2", "3", "4", "5", "6", "7", "8", "9" };
/* ffox, programming1, term, music, steam, folder, play icon, document, message  */
static const char *tagsalt[] = { "", "{}", "$", "", "", "", "", "", "" };

static const char *upvol[] = {"/opt/instantos/menus/dm/p.sh", "+5", NULL};
static const char *downvol[] = {"/opt/instantos/menus/dm/p.sh", "-5", NULL};
static const char *mutevol[] = {"/opt/instantos/menus/dm/p.sh", "m", NULL};

static const char *upbright[] = {"/opt/instantos/menus/dm/b.sh", "+", NULL};
static const char *downbright[] = {"/opt/instantos/menus/dm/b.sh", "-", NULL};


static const Rule rules[] = {
	/* xprop(1):
	 *	WM_CLASS(STRING) = instance, class
	 *	WM_NAME(STRING) = title
	 */
	/* class      instance    title       tags mask     isfloating   monitor */
	{"Gimp", 	  NULL,       NULL,       0,            1,           -1},
	{"pavucontrol", NULL,     NULL,       0,            1,           -1},
};

/* layout(s) */
static const float mfact = 0.55;  /* factor of master area size [0.05..0.95] */
static const int nmaster = 1;	 /* number of clients in master area */
static const int resizehints = 1; /* 1 means respect size hints in tiled resizals */

#include "tcl.c"
#include "layouts.c"

#include "gridmode.c"
static const Layout layouts[] = {
	/* symbol     arrange function */
	{ "+",      tile },    /* first entry is default */
	{ "#",      grid },
	{ "-",      NULL },    /* no layout function means floating behavior */
	{ "[M]",      monocle },
	{ "|||",      tcl },
	{ "H[]",      deck },
	{ NULL,       NULL },
};

/* key definitions */
#define MODKEY Mod4Mask
#define TAGKEYS(KEY, TAG)                                          \
	{MODKEY, KEY, view, {.ui = 1 << TAG}},                         \
		{MODKEY | ControlMask, KEY, toggleview, {.ui = 1 << TAG}}, \
		{MODKEY | ShiftMask, KEY, tag, {.ui = 1 << TAG}},          \
		{MODKEY | ControlMask | ShiftMask, KEY, toggletag, {.ui = 1 << TAG}},


#define SHCMD(cmd)                                           \
	{                                                        \
		.v = (const char *[]) { "/bin/sh", "-c", cmd, NULL } \
	}

/* commands */
static char instantmenumon[2] = "0"; /* component of instantmenucmd, manipulated in spawn() */
static const char *instantmenucmd[] = {"instantmenu_run", NULL};
static const char *roficmd[] = {"rofi", "-show", "run", NULL};
static const char *instantmenustcmd[] = {"instantmenu_run_st", NULL};
static const char *termcmd[] = {"urxvt", NULL};
static const char *instantassistcmd[] = {"instantassist", NULL};
static const char *nautiluscmd[] = {"nautilus", NULL};
static const char *slockcmd[] = {"ilock", NULL};
static const char *slockmcmd[] = {"ilock", "dmenu", NULL};
static const char *instantswitchcmd[] = {"instantswitch", NULL};
static const char *instantshutdowncmd[] = {"instantshutdown", NULL};
static const char *notifycmd[] = {"instantnotify", NULL};
static const char *rangercmd[] = { "urxvt", "-e", "ranger", NULL };
static const char *panther[] = { "appmenu", NULL};
static const char *pavucontrol[] = { "pavucontrol", NULL};
static const char  *clickcmd[] = { "autoclicker", NULL };

static const char *spoticli[] = { "spoticli", "m", NULL};
static const char *spotiprev[] = { "spoticli", "p", NULL};
static const char *spotinext[] = { "spoticli", "n", NULL};

#include "push.c"

static Key keys[] = {
	/* modifier                     key        function        argument */
	{MODKEY, XK_r, spawn, {.v = rangercmd } },
	{MODKEY, XK_n, spawn, {.v = nautiluscmd } },
	{MODKEY, XK_q, spawn, {.v = instantshutdowncmd } },
	{MODKEY, XK_y, spawn, {.v = panther} },
	{MODKEY, XK_a, spawn, {.v = instantassistcmd} },
	{MODKEY, XK_w, setoverlay, {0} },
	{MODKEY | ControlMask, XK_w, createoverlay, {0} },
	{MODKEY, XK_g, spawn, {.v = notifycmd} },
	{MODKEY | ControlMask, XK_space, spawn, {.v = instantmenucmd}},
	{MODKEY, XK_space, spawn, {.v = roficmd}},
	{MODKEY, XK_minus, spawn, {.v = instantmenustcmd}},
	{MODKEY, XK_x, spawn, {.v = instantswitchcmd}},
	{Mod1Mask, XK_Tab, spawn, {.v = instantswitchcmd}},
	{MODKEY | ControlMask, XK_l, spawn, {.v = slockcmd}},
	{MODKEY | ControlMask, XK_h, hidewin, {0}},
	{MODKEY | Mod1Mask, XK_h, unhideall, {0}},
	{MODKEY | Mod1Mask, XK_l, spawn, {.v = slockmcmd}},
	{MODKEY, XK_Return, spawn, {.v = termcmd}},
	{MODKEY, XK_b, togglebar, {0}},
	{MODKEY, XK_j, focusstack, {.i = +1}},
	{MODKEY, XK_Down, focusstack, {.i = +1}},
	{MODKEY, XK_k, focusstack, {.i = -1}},
	{MODKEY, XK_Up, focusstack, {.i = -1}},
	{MODKEY|ControlMask, XK_j, pushdown, {0} },
	{MODKEY|ControlMask, XK_k, pushup, {0} },
	{MODKEY, XK_s, togglealttag, {0} },
	{MODKEY|ShiftMask, XK_f, togglefakefullscreen, {0} },
	{MODKEY|ShiftMask, XK_w, warpfocus, {0} },
	{MODKEY|Mod1Mask, XK_w, centerwindow, {0} },
	{MODKEY|ShiftMask, XK_s, toggleshowtags, {0} },
	{MODKEY, XK_i, incnmaster, {.i = +1}},
	{MODKEY, XK_d, incnmaster, {.i = -1}},
	{MODKEY, XK_h, setmfact, {.f = -0.05}},
	{MODKEY, XK_l, setmfact, {.f = +0.05}},
	{MODKEY | ShiftMask, XK_Return, zoom, {0}},
	{MODKEY, XK_Tab, view, {0}},
	{MODKEY | ShiftMask, XK_c, killclient, {0}},
	{MODKEY, XK_t, setlayout, {.v = &layouts[0]}},
	{MODKEY, XK_f, setlayout, {.v = &layouts[2]}},
	{MODKEY, XK_m, setlayout, {.v = &layouts[3]}},
	{MODKEY, XK_c, setlayout, {.v = &layouts[1]}},

	{MODKEY,                       XK_Left,   viewtoleft,     {0}},
	{MODKEY,                       XK_Right,  viewtoright,    {0}},

	{MODKEY|ControlMask,           XK_Left,   viewleftclient,     {0}},
	{MODKEY|ControlMask,           XK_Right,  viewrightclient,    {0}},

	{MODKEY|ShiftMask,             XK_Left,   tagtoleft,      {0}},
	{MODKEY|ShiftMask,             XK_Right,  tagtoright,     {0}},

	{MODKEY|ShiftMask,				XK_j,  	moveresize,	{.i = 0}},
	{MODKEY|ShiftMask,				XK_k,  	moveresize,	{.i = 1}},
	{MODKEY|ShiftMask,				XK_l,  	moveresize,	{.i = 2}},
	{MODKEY|ShiftMask,				XK_h,  	moveresize,	{.i = 3}},
	
	{MODKEY|Mod1Mask,				XK_j,  	keyresize,	{.i = 0}},
	{MODKEY|Mod1Mask,				XK_k,  	keyresize,	{.i = 1}},
	{MODKEY|Mod1Mask,				XK_l,  	keyresize,	{.i = 2}},
	{MODKEY|Mod1Mask,				XK_h,  	keyresize,	{.i = 3}},


	{MODKEY|ControlMask,		XK_comma,  cyclelayout,    {.i = -1 } },
	{MODKEY|ControlMask,           XK_period, cyclelayout,    {.i = +1 } },
	{MODKEY, XK_p, setlayout, {0}},
	{MODKEY | ShiftMask, XK_space, togglefloating, {0}},
	{MODKEY, XK_0, view, {.ui = ~0}},
	{MODKEY | ShiftMask, XK_0, tag, {.ui = ~0}},
	{MODKEY, XK_comma, focusmon, {.i = -1}},
	{MODKEY, XK_period, focusmon, {.i = +1}},
	{MODKEY | ShiftMask, XK_comma, tagmon, {.i = -1}},
	{MODKEY | ShiftMask, XK_period, tagmon, {.i = +1}},
	TAGKEYS(XK_1, 0)
	TAGKEYS(XK_2, 1)
	TAGKEYS(XK_3, 2)
	TAGKEYS(XK_4, 3)
	TAGKEYS(XK_5, 4)
	TAGKEYS(XK_6, 5)
	TAGKEYS(XK_7, 6)
	TAGKEYS(XK_8, 7)
	TAGKEYS(XK_9, 8){MODKEY | ShiftMask, XK_q, quit, {0}},
	{0, XF86XK_AudioLowerVolume, spawn, {.v = downvol}},
	{0, XF86XK_AudioMute, spawn, {.v = mutevol}},
	{0, XF86XK_AudioRaiseVolume, spawn, {.v = upvol}},
	{0, XF86XK_AudioPlay, spawn, {.v = spoticli}},
	{0, XF86XK_AudioNext, spawn, {.v = spotinext}},
	{0, XF86XK_AudioPrev, spawn, {.v = spotiprev}},
	{ MODKEY, XK_o, winview, {0} },

};

/* button definitions */
/* click can be ClkTagBar, ClkLtSymbol, ClkStatusText, ClkWinTitle, ClkClientWin, or ClkRootWin */
static Button buttons[] = {
	/* click                event mask      button          function        argument */
	{ ClkLtSymbol,          0,              Button1,        setlayout,      {0} },
	{ ClkLtSymbol,          0,              Button3,        setlayout,      {.v = &layouts[2]} },
	{ ClkWinTitle,          0,              Button1,        togglewin,      {0} },
	{ ClkWinTitle,          MODKEY,         Button1,        setoverlay,      {0} },
	{ ClkWinTitle,          MODKEY,         Button3,        spawn,      {.v = notifycmd } },
	{ ClkWinTitle,          0,              Button2,        closewin,      {0} },
	{ ClkWinTitle,          0,              Button3,        zoom,           {0} },
	{ ClkWinTitle,          0,              Button5,        focusstack,     {.i = +1} },
	{ ClkWinTitle,          0,              Button4,        focusstack,     {.i = -1} },
	{ ClkWinTitle,          ShiftMask,      Button5,        pushdown,       {0} },
	{ ClkWinTitle,          ShiftMask,      Button4,        pushup,         {0} },
	{ ClkStatusText,        0,              Button2,        spawn,          {.v = termcmd } },
	{ ClkStatusText,        0,              Button4,        spawn,          {.v = upvol } },
	{ ClkStatusText,        0,              Button5,        spawn,          {.v = downvol } },
	{ ClkStatusText,        MODKEY,         Button2,        spawn,          {.v = mutevol } },
	{ ClkStatusText,        0,              Button1,        spawn,          {.v = panther } },
	{ ClkStatusText,        MODKEY|ShiftMask,Button1,       spawn,          {.v = pavucontrol } },
	{ ClkStatusText,        MODKEY,         Button3,       spawn,           {.v = spoticli } },
	{ ClkStatusText,        MODKEY,         Button4,        spawn,          {.v = upbright } },
	{ ClkStatusText,        MODKEY,         Button5,        spawn,          {.v = downbright } },
	{ ClkRootWin,           0,              Button1,        spawn,          {.v = panther } },
	{ ClkRootWin,           MODKEY,         Button1,        setoverlay,      {0} },
	{ ClkRootWin,           0,              Button3,        spawn,          {.v = roficmd } },
	{ ClkRootWin,           0,              Button2,        spawn,          {.v = instantmenucmd } },
	{ ClkClientWin,         MODKEY,         Button1,        movemouse,      {0} },
	{ ClkClientWin,         MODKEY,         Button2,        togglefloating, {0} },
	{ ClkClientWin,         MODKEY,         Button3,        resizemouse,    {0} },
	{ ClkTagBar,            0,              Button1,        view,           {0} },
	{ ClkTagBar,            0,              Button5,        viewtoright,    {0} },
	{ ClkTagBar,            MODKEY,         Button4,        viewleftclient, {0} },
	{ ClkTagBar,            MODKEY,         Button5,        viewrightclient,{0} },
	{ ClkTagBar,            0,              Button4,        viewtoleft,     {0} },
	{ ClkTagBar,            0,              Button3,        toggleview,     {0} },
	{ ClkTagBar,            MODKEY,         Button1,        tag,            {0} },
	{ ClkTagBar,            MODKEY,         Button3,        toggletag,      {0} },
};

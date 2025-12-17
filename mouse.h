/* See LICENSE file for copyright and license details. */

#ifndef MOUSE_H
#define MOUSE_H

#include "instantwm.h"

/* Mouse interaction constants */
#define MIN_WINDOW_SIZE 50        /* Minimum width/height for windows */
#define RESIZE_BORDER_ZONE 30     /* Border detection zone in pixels */
#define DRAG_THRESHOLD 5          /* Minimum movement to trigger drag */
#define MAX_UNMAXIMIZE_OFFSET 100 /* Tolerance for unmaximize detection */
#define GESTURE_THRESHOLD 200     /* Vertical movement for gesture actions */
#define OVERLAY_ZONE_WIDTH 50     /* Width of overlay trigger zone */
#define SLOP_MARGIN 40            /* Margin for slop selection validation */
#define REFRESH_RATE_HI 240       /* High refresh rate (double draw) */
#define REFRESH_RATE_LO 120       /* Standard refresh rate */
#define REFRESH_RATE_DRAG 60      /* Refresh rate for drag operations */
#define KEYCODE_ESCAPE 9          /* X11 keycode for Escape key */

void movemouse(const Arg *arg);
void gesturemouse(const Arg *arg);
int hoverresizemouse(const Arg *arg);
int isinresizeborder(void);
void window_title_mouse_handler(const Arg *arg);
void handle_window_title_right_mouse(const Arg *arg);
void drawwindow(const Arg *arg);
void dragtag(const Arg *arg);
void forceresizemouse(const Arg *arg);
void resizemouse(const Arg *arg);
void resizeaspectmouse(const Arg *arg);

/* Helper functions for drawwindow */
int parse_slop_output(const char *output, int dimensions[4]);
int is_valid_window_size(int x, int y, int width, int height, Client *c);
void handle_monitor_switch(Client *c, int x, int y, int width, int height);
void handle_client_monitor_switch(Client *c);
void apply_window_resize(Client *c, int x, int y, int width, int height);

#endif

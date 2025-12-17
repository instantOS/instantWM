/* See LICENSE file for copyright and license details. */

#ifndef FOCUS_H
#define FOCUS_H

#include "instantwm.h"

/* Focus navigation directions */
enum {
    FocusDirUp = 0,    /* Navigate to window above */
    FocusDirRight = 1, /* Navigate to window on right */
    FocusDirDown = 2,  /* Navigate to window below */
    FocusDirLeft = 3   /* Navigate to window on left */
};

void direction_focus(const Arg *arg);
void focus_last_client(const Arg *arg);
void warp(const Client *c);
void forcewarp(const Client *c);
void warpinto(const Client *c);
void warp_cursor_to_client(const Client *c);
void warp_to_focus();

#endif

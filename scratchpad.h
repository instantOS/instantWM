/* See LICENSE file for copyright and license details. */

#ifndef SCRATCHPAD_H
#define SCRATCHPAD_H

#include "instantwm.h"

/* Find a scratchpad client by name (scans all monitors) */
Client *scratchpad_find(const char *name);

/* Check if any scratchpad is visible on this monitor */
int scratchpad_any_visible(Monitor *m);

/* Identify a newly managed client as a scratchpad (by WM_CLASS) */
void scratchpad_identify_client(Client *c);

/* IPC / keybind actions (all take arg->v = "name") */
void scratchpad_make(const Arg *arg);
void scratchpad_unmake(const Arg *arg);
void scratchpad_toggle(const Arg *arg);
void scratchpad_show(const Arg *arg);
void scratchpad_hide(const Arg *arg);
void scratchpad_status(const Arg *arg);

#endif

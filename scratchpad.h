/* See LICENSE file for copyright and license details. */

#ifndef SCRATCHPAD_H
#define SCRATCHPAD_H

#include "instantwm.h"

void updatescratchvisible(Monitor *m);
Client *findnamedscratchpad(const char *name);
void makescratchpad(const Arg *arg);
void togglescratchpad(const Arg *arg);
void createscratchpad(const Arg *arg);
void showscratchpad(const Arg *arg);
void hidescratchpad(const Arg *arg);
void scratchpadstatus(const Arg *arg);

#endif

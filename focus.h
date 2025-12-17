/* See LICENSE file for copyright and license details. */

#ifndef FOCUS_H
#define FOCUS_H

#include "instantwm.h"

void direction_focus(const Arg *arg);
void focus_last_client(const Arg *arg);
void warp(const Client *c);
void forcewarp(const Client *c);
void warpinto(const Client *c);
void warp_focus();

#endif

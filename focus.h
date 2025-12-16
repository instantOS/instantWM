/* See LICENSE file for copyright and license details. */

#ifndef FOCUS_H
#define FOCUS_H

#include "instantwm.h"

void directionfocus(const Arg *arg);
void focuslastclient(const Arg *arg);
void warp(const Client *c);
void forcewarp(const Client *c);
void warpinto(const Client *c);
void warpfocus();

#endif

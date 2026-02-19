/* See LICENSE file for copyright and license details. */

#ifndef ANIMATION_H
#define ANIMATION_H

#include "instantwm.h"

double easeOutCubic(double t);
void checkanimate(Client *c, int x, int y, int w, int h, int frames,
                  int resetpos);
void animateclient(Client *c, int x, int y, int w, int h, int frames,
                   int resetpos);
void animleft(const Arg *arg);
void animright(const Arg *arg);

#endif

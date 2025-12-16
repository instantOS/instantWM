/* See LICENSE file for copyright and license details. */

#ifndef TOGGLES_H
#define TOGGLES_H

#include "instantwm.h"

void togglealttag(const Arg *arg);
void togglesticky(const Arg *arg);
void toggleprefix(const Arg *arg);
void toggleanimated(const Arg *arg);
void setborderwidth(const Arg *arg);
void togglefocusfollowsmouse(const Arg *arg);
void togglefocusfollowsfloatmouse(const Arg *arg);
void toggledoubledraw(const Arg *arg);
void togglefakefullscreen(const Arg *arg);
void togglelocked(const Arg *arg);
void toggleshowtags(const Arg *arg);
void togglebar(const Arg *arg);
void ctrltoggle(int *value, int arg);
void alttabfree(const Arg *arg);

#endif

/* See LICENSE file for copyright and license details. */

#ifndef KEYBOARD_H
#define KEYBOARD_H

#include "instantwm.h"

void keyrelease(XEvent *e);
void grabkeys(void);
void keypress(XEvent *e);
void uppress(const Arg *arg);
void downpress(const Arg *arg);
void upkey(const Arg *arg);
void downkey(const Arg *arg);
void spacetoggle(const Arg *arg);

#endif

/* See LICENSE file for copyright and license details. */

#ifndef MOUSE_H
#define MOUSE_H

#include "instantwm.h"

void movemouse(const Arg *arg);
void gesturemouse(const Arg *arg);
int resizeborder(const Arg *arg);
int isinresizeborder(void);
void dragmouse(const Arg *arg);
void dragrightmouse(const Arg *arg);
void drawwindow(const Arg *arg);
void dragtag(const Arg *arg);
void forceresizemouse(const Arg *arg);
void resizemouse(const Arg *arg);
void resizeaspectmouse(const Arg *arg);

#endif

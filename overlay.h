/* See LICENSE file for copyright and license details. */

#ifndef OVERLAY_H
#define OVERLAY_H

#include "instantwm.h"

int overlayexists(void);
void createoverlay(const Arg *arg);
void resetoverlay(void);
void showoverlay(const Arg *arg);
void hideoverlay(const Arg *arg);
void setoverlay(const Arg *arg);
void setoverlaymode(int mode);
void resetoverlaysize(void);

#endif

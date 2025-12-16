/* See LICENSE file for copyright and license details. */

#ifndef OVERLAY_H
#define OVERLAY_H

#include "instantwm.h"

int overlayexists(void);
void createoverlay(void);
void resetoverlay(void);
void showoverlay(void);
void hideoverlay(void);
void setoverlay(void);
void setoverlaymode(int mode);
void resetoverlaysize(void);

#endif

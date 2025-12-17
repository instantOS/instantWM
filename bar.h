/* See LICENSE file for copyright and license details. */

#ifndef BAR_H
#define BAR_H

#include "instantwm.h"

int drawstatusbar(Monitor *m, int bh, char *stext);
void drawbar(Monitor *m);
void drawbars(void);
void resetbar(void);
void updatestatus(void);
void clickstatus(const Arg *arg);
void updatebarpos(Monitor *m);
int get_blw(Monitor *m);
void resizebarwin(Monitor *m);
void togglebar(const Arg *arg);
void updatebars(void);

#endif

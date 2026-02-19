/* See LICENSE file for copyright and license details. */

#ifndef TAGS_H
#define TAGS_H

#include "instantwm.h"

void view(const Arg *arg);
void toggleview(const Arg *arg);
void tag(const Arg *arg);
void toggletag(const Arg *arg);
void tagmon(const Arg *arg);
void tagall(const Arg *arg);
void followtag(const Arg *arg);
void swaptags(const Arg *arg);
void followview(const Arg *arg);
void tagtoleft(const Arg *arg);
void tagtoright(const Arg *arg);
void viewtoleft(const Arg *arg);
void viewtoright(const Arg *arg);
void moveleft(const Arg *arg);
void moveright(const Arg *arg);
void shiftview(const Arg *arg);
void nametag(const Arg *arg);
void resetnametag(const Arg *arg);
void winview(const Arg *arg);
int gettagwidth(void);
int get_tag_at_x(int ix);
void toggle_overview(const Arg *arg);
void toggle_fullscreen_overview(const Arg *arg);
void lastview(const Arg *arg);
void resetsticky(Client *c);

#endif

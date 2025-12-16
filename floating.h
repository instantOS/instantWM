/* See LICENSE file for copyright and license details. */

#ifndef FLOATING_H
#define FLOATING_H

#include "instantwm.h"

void resetsnap(Client *c);
void saveallfloating(Monitor *m);
void restoreallfloating(Monitor *m);
void applysnap(Client *c, Monitor *m);
int checkfloating(Client *c);
int visible(Client *c);
void changesnap(Client *c, int snapmode);
void tempfullscreen(const Arg *arg);
void savefloating(Client *c);
void restorefloating(Client *c);
void savebw(Client *c);
void restorebw(Client *c);
void applysize(Client *c);
void togglefloating(const Arg *arg);
void changefloating(Client *c);
void centerwindow(const Arg *arg);
void scaleclient(Client *c, int scale);
void upscaleclient(const Arg *arg);
void downscaleclient(const Arg *arg);
void moveresize(const Arg *arg);
void keyresize(const Arg *arg);

#endif

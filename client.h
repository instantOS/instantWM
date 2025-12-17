/* See LICENSE file for copyright and license details. */

#ifndef CLIENT_H
#define CLIENT_H

#include "instantwm.h"

void attach(Client *c);
void attachstack(Client *c);
void detach(Client *c);
void detachstack(Client *c);
Client *nexttiled(Client *c);
void pop(Client *c);
Client *wintoclient(Window w);
void setclientstate(Client *c, long state);
void setclienttagprop(Client *c);
int sendevent(Window w, Atom proto, int mask, long d0, long d1, long d2,
              long d3, long d4);
void configure(Client *c);
void setfocus(Client *c);
void unfocus(Client *c, int setfocus);
void showhide(Client *c);
void show(Client *c);
void hide(Client *c);
void resize(Client *c, int x, int y, int w, int h, int interact);
void resizeclient(Client *c, int x, int y, int w, int h);

#endif

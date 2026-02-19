/* See LICENSE file for copyright and license details. */

#ifndef EVENTS_H
#define EVENTS_H

#include "instantwm.h"

/* Event handlers that don't depend on static config arrays */
void clientmessage(XEvent *e);
void configurenotify(XEvent *e);
void configurerequest(XEvent *e);
void destroynotify(XEvent *e);
void enternotify(XEvent *e);
void expose(XEvent *e);
void focusin(XEvent *e);
void mappingnotify(XEvent *e);
void maprequest(XEvent *e);
void motionnotify(XEvent *e);
void propertynotify(XEvent *e);
void resizerequest(XEvent *e);
void unmapnotify(XEvent *e);
void leavenotify(XEvent *e);

#endif

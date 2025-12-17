/* See LICENSE file for copyright and license details. */

#include "monitors.h"
#include "scratchpad.h"
#include "tags.h"
#include "util.h"
#include "focus.h"
#include <stdlib.h>

/* extern declarations for variables from instantwm.c */
extern Display *dpy;
extern Monitor *mons;
extern Monitor *selmon;
extern Window root;

void cleanupmon(Monitor *mon) {
    Monitor *m;

    if (mon == mons)
        mons = mons->next;
    else {
        for (m = mons; m && m->next != mon; m = m->next)
            ;
        m->next = mon->next;
    }
    XUnmapWindow(dpy, mon->barwin);
    XDestroyWindow(dpy, mon->barwin);
    free(mon);
}

Monitor *dirtomon(int dir) {
    Monitor *m = NULL;

    if (dir > 0) {
        if (!(m = selmon->next))
            m = mons;
    } else if (selmon == mons)
        for (m = mons; m->next; m = m->next)
            ;
    else
        for (m = mons; m->next != selmon; m = m->next)
            ;
    return m;
}

void followmon(const Arg *arg) {
    Client *c;
    if (!selmon->sel)
        return;
    c = selmon->sel;
    tagmon(arg);
    selmon = c->mon;
    focus(NULL);
    focus(c);
    XRaiseWindow(dpy, c->win);
    warp_cursor_to_client(c);
}

void focusmon(const Arg *arg) {
    Monitor *m;

    if (!mons->next)
        return;
    if ((m = dirtomon(arg->i)) == selmon)
        return;
    unfocus(selmon->sel, 0);
    selmon = m;
    focus(NULL);
}

void focusnmon(const Arg *arg) {
    Monitor *m;
    int i;

    if (!mons->next)
        return;

    m = mons;

    for (i = 0; i < arg->i; ++i) {
        if (m->next) {
            m = m->next;
        } else {
            break;
        }
    }

    unfocus(selmon->sel, 0);
    selmon = m;
    focus(NULL);
}

Monitor *recttomon(int x, int y, int w, int h) {
    Monitor *m, *r = selmon;
    int a, area = 0;
    for (m = mons; m; m = m->next)
        if ((a = INTERSECT(x, y, w, h, m)) > area) {
            area = a;
            r = m;
        }
    return r;
}

void sendmon(Client *c, Monitor *m) {
    int isscratchpad = 0;
    Monitor *prevmon = selmon;
    if (c->mon == m)
        return;

    prevmon = selmon;

    unfocus(c, 1);
    detach(c);
    detachstack(c);
    c->mon = m;
    // make scratchpad windows reappear on the other monitor scratchpad
    if (c->tags != (SCRATCHPAD_MASK)) {
        c->tags = m->tagset[m->seltags]; /* assign tags of target monitor */
        resetsticky(c);
    } else {
        isscratchpad = 1;
    }
    attach(c);
    attachstack(c);
    setclienttagprop(c);
    focus(NULL);
    if (!c->isfloating)
        arrange(NULL);
    if (isscratchpad && !c->mon->scratchvisible) {
        unfocus(selmon->sel, 0);
        selmon = m;
        togglescratchpad(NULL);
        focus(NULL);
        unfocus(selmon->sel, 0);
        selmon = prevmon;
        focus(NULL);
    }
}

Monitor *wintomon(Window w) {
    int x, y;
    Client *c;
    Monitor *m;

    if (w == root && getrootptr(&x, &y))
        return recttomon(x, y, 1, 1);
    for (m = mons; m; m = m->next)
        if (w == m->barwin)
            return m;
    if ((c = wintoclient(w)))
        return c->mon;
    return selmon;
}

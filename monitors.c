/* See LICENSE file for copyright and license details. */

#include "monitors.h"
#include "focus.h"
#include "globals.h"
#include "scratchpad.h"
#include "tags.h"
#include "util.h"
#include "layouts.h" // For createmon
#include "client.h" // For sendmon, etc
#include "bar.h" // For cleanupmon
#include <stdlib.h>
#include <string.h>

#ifdef XINERAMA
#include <X11/extensions/Xinerama.h>
#endif /* XINERAMA */

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

Monitor *createmon(void) {
    Monitor *m;
    unsigned int i;

    m = ecalloc(1, sizeof(Monitor));
    m->tagset[0] = m->tagset[1] = 1;
    m->mfact = mfact;
    m->nmaster = nmaster;
    m->showbar = showbar;
    m->topbar = topbar;
    m->clientcount = 0;
    m->overlaymode = 0;
    m->scratchvisible = 0;
    m->lt[0] = &layouts[3];
    m->lt[1] = (Layout *)&layouts[0];
    strncpy(m->ltsymbol, layouts[0].symbol, sizeof m->ltsymbol);
    m->pertag = ecalloc(1, sizeof(Pertag));
    m->pertag->current_tag = m->pertag->prevtag = 1;

    for (i = 0; i < MAX_TAGS; i++) {
        m->pertag->nmasters[i] = m->nmaster;
        m->pertag->mfacts[i] = m->mfact;

        m->pertag->ltidxs[i][0] = m->lt[1];
        m->pertag->ltidxs[i][1] = m->lt[0];
        m->pertag->sellts[i] = m->sellt;

        m->pertag->showbars[i] = m->showbar;
    }

    return m;
}

#ifdef XINERAMA
static int isuniquegeom(XineramaScreenInfo *unique, size_t n,
                        XineramaScreenInfo *info) {
    while (n--)
        if (unique[n].x_org == info->x_org && unique[n].y_org == info->y_org &&
            unique[n].width == info->width && unique[n].height == info->height)
            return 0;
    return 1;
}
#endif /* XINERAMA */

int updategeom(void) {
    int dirty = 0;

#ifdef XINERAMA
    if (XineramaIsActive(dpy)) {
        int i, j, n, nn;
        Client *c;
        Monitor *m;
        XineramaScreenInfo *info = XineramaQueryScreens(dpy, &nn);
        XineramaScreenInfo *unique = NULL;

        for (n = 0, m = mons; m; m = m->next, n++)
            ;
        /* only consider unique geometries as separate screens */
        unique = ecalloc(nn, sizeof(XineramaScreenInfo));
        for (i = 0, j = 0; i < nn; i++)
            if (isuniquegeom(unique, j, &info[i]))
                memcpy(&unique[j++], &info[i], sizeof(XineramaScreenInfo));
        XFree(info);
        nn = j;
        for (i = n; i < nn; i++) {
            for (m = mons; m && m->next; m = m->next)
                ;
            if (m)
                m->next = createmon();
            else
                mons = createmon();
        }
        for (i = 0, m = mons; i < nn && m; m = m->next, i++)
            if (i >= n || unique[i].x_org != m->mx ||
                unique[i].y_org != m->my || unique[i].width != m->mw ||
                unique[i].height != m->mh) {
                dirty = 1;
                m->num = i;
                m->mx = m->wx = unique[i].x_org;
                m->my = m->wy = unique[i].y_org;
                m->mw = m->ww = unique[i].width;
                m->mh = m->wh = unique[i].height;
                updatebarpos(m);
            }
        for (i = nn; i < n; i++) {
            for (m = mons; m && m->next; m = m->next)
                ;
            while ((c = m->clients)) {
                dirty = 1;
                m->clients = c->next;
                detachstack(c);
                c->mon = mons;
                attach(c);
                attachstack(c);
            }
            if (m == selmon)
                selmon = mons;
            cleanupmon(m);
        }
        free(unique);
    } else
#endif /* XINERAMA */
    {  /* default monitor setup */
        if (!mons)
            mons = createmon();
        if (mons->mw != sw || mons->mh != sh) {
            dirty = 1;
            mons->mw = mons->ww = sw;
            mons->mh = mons->wh = sh;
            updatebarpos(mons);
        }
    }
    if (dirty) {
        selmon = mons;
        selmon = wintomon(root);
    }
    return dirty;
}

/* See LICENSE file for copyright and license details. */

#include <string.h>

#include "floating.h"
#include "focus.h"
#include "globals.h"
#include "layouts.h"
#include "scratchpad.h"

#define SCRATCHPAD_CLASS_PREFIX "scratchpad_"
#define SCRATCHPAD_CLASS_PREFIX_LEN 11

/* Find a scratchpad client by name (scans all monitors). */
Client *scratchpad_find(const char *name) {
    if (!name || name[0] == '\0')
        return NULL;

    Client *c;
    Monitor *m;
    for (m = mons; m; m = m->next)
        for (c = m->clients; c; c = c->next)
            if (ISSCRATCHPAD(c) && strcmp(c->scratchpad_name, name) == 0)
                return c;
    return NULL;
}

/* Check if any scratchpad is visible (sticky) on this monitor. */
int scratchpad_any_visible(Monitor *m) {
    Client *c;
    for (c = m->clients; c; c = c->next)
        if (ISSCRATCHPAD(c) && c->issticky)
            return 1;
    return 0;
}

/* Called during manage() to detect scratchpad windows by WM_CLASS.
 * If WM_CLASS matches "scratchpad_<name>", populate c->scratchpad_name
 * and apply scratchpad invariants. */
void scratchpad_identify_client(Client *c) {
    XClassHint ch = {NULL, NULL};
    const char *match = NULL;

    if (!XGetClassHint(dpy, c->win, &ch))
        return;

    if (ch.res_class &&
        strncmp(ch.res_class, SCRATCHPAD_CLASS_PREFIX,
                SCRATCHPAD_CLASS_PREFIX_LEN) == 0 &&
        ch.res_class[SCRATCHPAD_CLASS_PREFIX_LEN] != '\0') {
        match = ch.res_class + SCRATCHPAD_CLASS_PREFIX_LEN;
    } else if (ch.res_name &&
               strncmp(ch.res_name, SCRATCHPAD_CLASS_PREFIX,
                       SCRATCHPAD_CLASS_PREFIX_LEN) == 0 &&
               ch.res_name[SCRATCHPAD_CLASS_PREFIX_LEN] != '\0') {
        match = ch.res_name + SCRATCHPAD_CLASS_PREFIX_LEN;
    }

    if (match) {
        snprintf(c->scratchpad_name, SCRATCHPAD_NAME_LEN, "%s", match);
        c->scratchpad_restore_tags = 0;
        c->tags = SCRATCHPAD_MASK;
        c->issticky = 1;
        c->isfloating = 1;
    }

    if (ch.res_class)
        XFree(ch.res_class);
    if (ch.res_name)
        XFree(ch.res_name);
}

/* Turn the currently focused window into a named scratchpad.
 * arg->v = "name". If name is empty, does nothing.
 * If a scratchpad with that name already exists, does nothing. */
void scratchpad_make(const Arg *arg) {
    const char *name = arg ? arg->v : NULL;
    if (!name || name[0] == '\0')
        return;
    if (!selmon->sel)
        return;

    /* Don't allow duplicate names */
    if (scratchpad_find(name))
        return;

    Client *c = selmon->sel;

    /* If it's already a scratchpad, just rename it */
    if (!ISSCRATCHPAD(c))
        c->scratchpad_restore_tags = c->tags;

    snprintf(c->scratchpad_name, SCRATCHPAD_NAME_LEN, "%s", name);
    c->tags = SCRATCHPAD_MASK;
    c->issticky = 0;

    if (!c->isfloating)
        setfloating(c, 0);

    focus(NULL);
    arrange(selmon);
}

/* Remove scratchpad status from the currently focused window,
 * restoring it to its previous tag. */
void scratchpad_unmake(const Arg *arg) {
    if (!selmon->sel)
        return;

    Client *c = selmon->sel;
    if (!ISSCRATCHPAD(c))
        return;

    c->scratchpad_name[0] = '\0';
    c->issticky = 0;

    if (c->scratchpad_restore_tags)
        c->tags = c->scratchpad_restore_tags;
    else
        c->tags = selmon->tagset[selmon->seltags];

    c->scratchpad_restore_tags = 0;
    arrange(selmon);
}

/* Show a named scratchpad on the current monitor.
 * arg->v = "name". */
void scratchpad_show(const Arg *arg) {
    const char *name = arg ? arg->v : NULL;
    if (!name || name[0] == '\0')
        return;

    Client *found = scratchpad_find(name);
    if (!found)
        return;

    found->issticky = 1;
    setfloating(found, 0);

    /* Move to current monitor if on a different one */
    if (found->mon != selmon) {
        detach(found);
        detachstack(found);
        found->mon = selmon;
        attach(found);
        attachstack(found);
    }

    focus(found);
    arrange(selmon);
    restack(selmon);
    if (focusfollowsmouse)
        warp_cursor_to_client(found);
}

/* Hide a named scratchpad.
 * arg->v = "name". */
void scratchpad_hide(const Arg *arg) {
    const char *name = arg ? arg->v : NULL;
    if (!name || name[0] == '\0')
        return;

    Client *found = scratchpad_find(name);
    if (!found || !found->issticky)
        return;

    found->issticky = 0;
    found->tags = SCRATCHPAD_MASK;
    focus(NULL);
    arrange(found->mon);
}

/* Toggle a named scratchpad: show if hidden, hide if visible.
 * arg->v = "name".
 * In overview layout, does nothing. */
void scratchpad_toggle(const Arg *arg) {
    const char *name = arg ? arg->v : NULL;
    if (!name || name[0] == '\0')
        return;

    if (&overviewlayout == tiling_layout_func(selmon))
        return;

    Client *found = scratchpad_find(name);
    if (!found)
        return;

    if (found->issticky)
        scratchpad_hide(arg);
    else
        scratchpad_show(arg);
}

/* Report scratchpad status via root window name.
 * arg->v = "name" → "ipc:scratchpad:<name>:<0|1>"
 * arg->v = "all"  → "ipc:scratchpads:<name1>=<0|1>,<name2>=<0|1>,..." */
void scratchpad_status(const Arg *arg) {
    const char *name = arg ? arg->v : NULL;

    if (name && name[0] != '\0' && strcmp(name, "all") != 0) {
        Client *found = scratchpad_find(name);
        int visible = found && found->issticky;
        char status[256];
        snprintf(status, sizeof(status), "ipc:scratchpad:%s:%d", name, visible);
        XStoreName(dpy, root, status);
        XFlush(dpy);
        return;
    }

    /* "all" or no name: list all known scratchpads */
    char status[1024];
    int offset = snprintf(status, sizeof(status), "ipc:scratchpads:");
    int first = 1;

    Client *c;
    Monitor *m;
    for (m = mons; m; m = m->next) {
        for (c = m->clients; c; c = c->next) {
            if (!ISSCRATCHPAD(c))
                continue;
            int n = snprintf(status + offset, sizeof(status) - offset,
                             "%s%s=%d", first ? "" : ",", c->scratchpad_name,
                             c->issticky ? 1 : 0);
            if (n > 0)
                offset += n;
            first = 0;
        }
    }

    if (first) {
        /* No scratchpads found at all */
        snprintf(status + offset, sizeof(status) - offset, "none");
    }

    XStoreName(dpy, root, status);
    XFlush(dpy);
}

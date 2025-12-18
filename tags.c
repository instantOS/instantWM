/* See LICENSE file for copyright and license details. */

#include "tags.h"
#include "animation.h"
#include "bar.h"
#include "floating.h"
#include "globals.h"
#include "instantwm.h"
#include "layouts.h"
#include "overlay.h"
#include "scratchpad.h"
#include "systray.h"
#include "toggles.h"
#include "util.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define MAX_TAGLEN 16

/* External declarations not covered by headers */
extern const char *tags_default[];

/* functions */

/**
 * Compute effective tag value with optional prefix modifier.
 *
 * The tag prefix feature allows accessing tags beyond the first 10 by
 * pressing a prefix key first. When tagprefix is set, the tag value
 * is shifted left by 10 bits (e.g., tag 1 becomes tag 11).
 *
 * @param arg: argument containing the base tag value in arg->ui
 * @return: the effective tag value (shifted if prefix is active)
 *
 * Side effect: clears tagprefix after use if it was set.
 */
int computeprefix(const Arg *arg) {
    if (tagprefix && arg->ui) {
        tagprefix = 0;
        return arg->ui << 10;
    } else {
        return arg->ui;
    }
}

void nametag(const Arg *arg) {
    int i;

    char *name = (char *)arg->v;

    if (strlen(name) >= MAX_TAGLEN)
        return;

    for (i = 0; i < numtags; i++) {
        if (selmon->tagset[selmon->seltags] & (1 << i)) {
            if (strlen(name) > 0)
                strcpy(tags[i], name);
            else
                strcpy(tags[i], tags_default[i]);
        }
    }
    tagwidth = gettagwidth();
    drawbars();
}

void resetnametag(const Arg *arg) {
    for (int i = 0; i < numtags; i++)
        strcpy((char *)&tags[i], tags_default[i]);
    tagwidth = gettagwidth();
    drawbars();
}

int gettagwidth() {
    int x = 0, i = 0, occupied_tags = 0;
    Client *c;

    for (c = selmon->clients; c; c = c->next)
        occupied_tags |= c->tags == 255 ? 0 : c->tags;

    do {
        // do not reserve space for vacant tags
        if (i >= 9)
            continue;
        if (selmon->showtags) {
            if (!(occupied_tags & 1 << i ||
                  selmon->tagset[selmon->seltags] & 1 << i))
                continue;
        }
        x += TEXTW(tags[i]);
    } while (++i < numtags);
    return x + startmenusize;
}

// return tag for indicator of given x coordinate
int getxtag(int ix) {
    int x, i, occupied_tags;
    Client *c;
    i = 0;
    occupied_tags = 0;
    x = startmenusize;
    for (c = selmon->clients; c; c = c->next)
        occupied_tags |= c->tags == 255 ? 0 : c->tags;

    do {
        // do not reserve space for vacant tags
        if (i >= 9)
            continue;
        if (selmon->showtags) {
            if (!(occupied_tags & 1 << i ||
                  selmon->tagset[selmon->seltags] & 1 << i))
                continue;
        }

        x += TEXTW(tags[i]);
        if (x >= ix) {
            return i;
        }
    } while (++i < numtags);
    return -1;
}

/**
 * Move the selected client to the specified tag(s).
 *
 * @param tagmask_bits: bitmask of target tag(s)
 */
static void setclienttag_impl(unsigned int tagmask_bits) {
    Client *c;
    if (selmon->sel && tagmask_bits & tagmask) {
        if (selmon->sel->tags == SCRATCHPAD_MASK)
            selmon->sel->issticky = 0;
        c = selmon->sel;
        selmon->sel->tags = tagmask_bits & tagmask;
        setclienttagprop(c);
        focus(NULL);
        arrange(selmon);
    }
}

/** Move the selected client to the tag specified by arg->ui. */
void tag(const Arg *arg) {
    int ui = computeprefix(arg);
    setclienttag_impl(ui);
}

void tagall(const Arg *arg) {
    Client *c;
    int ui = computeprefix(arg);
    if (PERTAG_CURRENT(selmon) == 0)
        return;
    if (selmon->sel && ui & tagmask) {
        for (c = selmon->clients; c; c = c->next) {
            if (!(c->tags & 1 << (PERTAG_CURRENT(selmon) - 1)))
                continue;
            if (c->tags == SCRATCHPAD_MASK)
                c->issticky = 0;
            c->tags = ui & tagmask;
        }
        focus(NULL);
        arrange(selmon);
    }
}

/**
 * Move the selected client to a tag and switch view to that tag.
 *
 * @param tagmask_bits: bitmask of target tag
 * @param preserve_prefix: if true, maintain tagprefix state after tag()
 */
static void followtag_impl(unsigned int tagmask_bits, int preserve_prefix) {
    if (!selmon->sel)
        return;
    Arg a = {.ui = tagmask_bits};
    tag(&a);
    if (preserve_prefix)
        tagprefix = 1;
    view(&a);
}

/**
 * Move the selected client to a tag and follow it there.
 * Wrapper around followtag_impl for keybinding compatibility.
 */
void followtag(const Arg *arg) {
    int ui = computeprefix(arg);
    followtag_impl(ui, tagprefix != 0);
}

/** Swap client tags between current tag and target tag. */
static void swap_client_tags(unsigned int current_tag, unsigned int newtag) {
    for (Client *c = selmon->clients; c != NULL; c = c->next) {
        if (selmon->overlay == c) {
            if (ISVISIBLE(c))
                hideoverlay(NULL);
            continue;
        }
        if ((c->tags & newtag) || (c->tags & current_tag))
            c->tags ^= current_tag ^ newtag;

        if (!c->tags)
            c->tags = newtag;
    }
}

/** Swap per-tag settings (layout, mfact, nmaster, etc.) between two tags. */
static void swap_pertag_settings(int target_tag_idx) {
    int current_idx = selmon->pertag->current_tag;

    /* Save current tag settings to temp variables */
    int tmpnmaster = PERTAG_NMASTER(selmon);
    float tmpmfact = PERTAG_MFACT(selmon);
    int tmpsellt = PERTAG_SELLT(selmon);
    const Layout *tmplt[2];
    tmplt[selmon->sellt] = PERTAG_LAYOUT(selmon);
    tmplt[selmon->sellt ^ 1] =
        selmon->pertag->ltidxs[PERTAG_CURRENT(selmon)][selmon->sellt ^ 1];
    int tmpshowbar = PERTAG_SHOWBAR(selmon);

    /* Copy target tag settings to current tag */
    selmon->pertag->nmasters[current_idx] =
        selmon->pertag->nmasters[target_tag_idx];
    selmon->pertag->mfacts[current_idx] =
        selmon->pertag->mfacts[target_tag_idx];
    selmon->pertag->sellts[current_idx] =
        selmon->pertag->sellts[target_tag_idx];
    selmon->pertag->ltidxs[current_idx][selmon->sellt] =
        selmon->pertag->ltidxs[target_tag_idx][selmon->sellt];
    selmon->pertag->ltidxs[current_idx][selmon->sellt ^ 1] =
        selmon->pertag->ltidxs[target_tag_idx][selmon->sellt ^ 1];
    selmon->pertag->showbars[current_idx] =
        selmon->pertag->showbars[target_tag_idx];

    /* Copy saved settings to target tag */
    selmon->pertag->nmasters[target_tag_idx] = tmpnmaster;
    selmon->pertag->mfacts[target_tag_idx] = tmpmfact;
    selmon->pertag->sellts[target_tag_idx] = tmpsellt;
    selmon->pertag->ltidxs[target_tag_idx][selmon->sellt] =
        tmplt[selmon->sellt];
    selmon->pertag->ltidxs[target_tag_idx][selmon->sellt ^ 1] =
        tmplt[selmon->sellt ^ 1];
    selmon->pertag->showbars[target_tag_idx] = tmpshowbar;
}

/**
 * Swap all clients and settings between the current tag and a target tag.
 * This exchanges both the window assignments and per-tag settings.
 */
void swaptags(const Arg *arg) {
    int ui = computeprefix(arg);
    unsigned int newtag = ui & tagmask;
    unsigned int current_tag = selmon->tagset[selmon->seltags];

    /* Validate: must be different tags, current must exist, must be single tag
     */
    if (newtag == current_tag || !current_tag ||
        (current_tag & (current_tag - 1)))
        return;

    /* Find target tag index */
    int target_idx;
    for (target_idx = 0; !(ui & 1 << target_idx); target_idx++)
        ;

    /* Swap clients between tags */
    swap_client_tags(current_tag, newtag);

    /* Update current tagset to target */
    selmon->tagset[selmon->seltags] = newtag;

    /* Swap per-tag settings */
    swap_pertag_settings(target_idx + 1);

    /* Update tag tracking */
    if (selmon->pertag->prevtag == target_idx + 1)
        selmon->pertag->prevtag = selmon->pertag->current_tag;
    selmon->pertag->current_tag = target_idx + 1;

    focus(NULL);
    arrange(selmon);
}

void followview(const Arg *arg) {
    if (!selmon->sel)
        return;
    Client *c = selmon->sel;
    c->tags = 1 << (selmon->pertag->prevtag - 1);
    view(&((Arg){.ui = 1 << (selmon->pertag->prevtag - 1)}));
    focus(c);
    arrange(selmon);
}

void resetsticky(Client *c) {
    if (!c->issticky)
        return;
    c->issticky = 0;
    c->tags = 1 << (PERTAG_CURRENT(selmon) - 1);
}

void tagmon(const Arg *arg) {
    if (!selmon->sel || !mons->next)
        return;

    if (selmon->sel->isfloating) {
        Client *c;
        float xfact, yfact;
        c = selmon->sel;
        xfact = (float)(c->x - selmon->mx) / selmon->ww;
        yfact = (float)(c->y - selmon->my) / selmon->wh;

        sendmon(selmon->sel, dirtomon(arg->i));
        c->x = c->mon->mx + c->mon->ww * xfact;
        c->y = c->mon->my + c->mon->wh * yfact;
        arrange(c->mon);
        XRaiseWindow(dpy, c->win);
    } else {
        sendmon(selmon->sel, dirtomon(arg->i));
    }
}

/**
 * Shift the selected client to an adjacent tag.
 *
 * @param direction: negative for left, positive for right
 * @param offset: number of tags to shift (default 1)
 */
static void shift_tag(int dir, int offset) {
    int oldx;
    Client *c;

    if (!selmon->sel)
        return;

    /* Handle overlay special case */
    if (selmon->sel == selmon->overlay) {
        setoverlaymode(dir == DirLeft ? 3 : OverlayRight);
        return;
    }

    /* Boundary checks */
    if (dir == DirLeft && PERTAG_CURRENT(selmon) == 1)
        return;
    if (dir == DirRight && PERTAG_CURRENT(selmon) == 20)
        return;

    c = selmon->sel;
    resetsticky(c);
    oldx = c->x;

    /* Animate the window sliding in the direction of movement */
    if (!c->isfloating && animated) {
        XRaiseWindow(dpy, c->win);
        int anim_offset = (selmon->mw / 10) * (dir == DirLeft ? -1 : 1);
        animateclient(c, c->x + anim_offset, c->y, 0, 0, 7, 0);
    }

    /* Shift the client's tag */
    int is_single_tag =
        __builtin_popcount(selmon->tagset[selmon->seltags] & tagmask) == 1;

    if (selmon->sel != NULL && is_single_tag) {
        if (dir == DirLeft && selmon->tagset[selmon->seltags] > 1) {
            selmon->sel->tags >>= offset;
            focus(NULL);
            arrange(selmon);
        } else if (dir == DirRight &&
                   (selmon->tagset[selmon->seltags] & (tagmask >> 1))) {
            selmon->sel->tags <<= offset;
            focus(NULL);
            arrange(selmon);
        }
    }
    c->x = oldx;
}

/** Move the selected client to the previous (left) tag. */
void tagtoleft(const Arg *arg) {
    int offset = (arg && arg->i) ? arg->i : 1;
    shift_tag(DirLeft, offset);
}

/** Move the selected client to the next (right) tag. */
void tagtoright(const Arg *arg) {
    int offset = (arg && arg->i) ? arg->i : 1;
    shift_tag(DirRight, offset);
}

void toggletag(const Arg *arg) {
    int ui = computeprefix(arg);
    unsigned int newtags;

    if (!selmon->sel)
        return;

    if (selmon->sel->tags == SCRATCHPAD_MASK) {
        tag(arg);
        return;
    }

    newtags = selmon->sel->tags ^ (ui & tagmask);
    if (newtags) {
        selmon->sel->tags = newtags;
        setclienttagprop(selmon->sel);
        focus(NULL);
        arrange(selmon);
    }
}

void toggleview(const Arg *arg) {
    unsigned int newtagset =
        selmon->tagset[selmon->seltags] ^ (arg->ui & tagmask);
    int i;

    if (newtagset) {
        selmon->tagset[selmon->seltags] = newtagset;

        if (newtagset == ~0) {
            selmon->pertag->prevtag = selmon->pertag->current_tag;
            selmon->pertag->current_tag = 0;
        }

        /* test if the user did not select the same tag */
        if (!(newtagset & 1 << (selmon->pertag->current_tag - 1))) {
            selmon->pertag->prevtag = selmon->pertag->current_tag;
            for (i = 0; !(newtagset & 1 << i); i++)
                ;
            selmon->pertag->current_tag = i + 1;
        }

        /* apply settings for this view */
        selmon->nmaster = PERTAG_NMASTER(selmon);
        selmon->mfact = PERTAG_MFACT(selmon);
        selmon->sellt = PERTAG_SELLT(selmon);
        selmon->lt[selmon->sellt] = PERTAG_LAYOUT(selmon);
        selmon->lt[selmon->sellt ^ 1] =
            selmon->pertag->ltidxs[PERTAG_CURRENT(selmon)][selmon->sellt ^ 1];

        if (selmon->showbar != PERTAG_SHOWBAR(selmon))
            togglebar(NULL);

        focus(NULL);
        arrange(selmon);
    }
}

void view(const Arg *arg) {

    int ui = computeprefix(arg);
    int i;
    printf("%d\n", (int)(arg->ui));

    selmon->seltags ^= 1; /* toggle sel tagset */
    if (ui & tagmask) {
        selmon->tagset[selmon->seltags] = ui & tagmask;

        if (ui == ~0) {
            selmon->pertag->prevtag = selmon->pertag->current_tag;
            selmon->pertag->current_tag = 0;
        } else {
            for (i = 0; !(ui & 1 << i); i++)
                ;
            if ((i + 1) == selmon->pertag->current_tag)
                return;
            selmon->pertag->prevtag = selmon->pertag->current_tag;
            selmon->pertag->current_tag = i + 1;
        }

        /* apply settings for this view */
        selmon->nmaster = PERTAG_NMASTER(selmon);
        selmon->mfact = PERTAG_MFACT(selmon);
        selmon->sellt = PERTAG_SELLT(selmon);
        selmon->lt[selmon->sellt] = PERTAG_LAYOUT(selmon);
        selmon->lt[selmon->sellt ^ 1] =
            selmon->pertag->ltidxs[PERTAG_CURRENT(selmon)][selmon->sellt ^ 1];

        if (selmon->showbar != PERTAG_SHOWBAR(selmon))
            togglebar(NULL);

        focus(NULL);
        arrange(selmon);
    }
}

static void viewscroll(const Arg *arg, int dir) {
    int i;
    unsigned int tagmask_val = tagmask;

    if (dir == DirLeft) {
        if (PERTAG_CURRENT(selmon) == 1)
            return;
        if (__builtin_popcount(selmon->tagset[selmon->seltags] & tagmask_val) ==
                1 &&
            selmon->tagset[selmon->seltags] > 1) {
            selmon->seltags ^= 1; /* toggle sel tagset */
            selmon->tagset[selmon->seltags] =
                selmon->tagset[selmon->seltags ^ 1] >> 1;
        } else {
            return;
        }
    } else { // DirRight
        if (PERTAG_CURRENT(selmon) == 20)
            return;
        if (__builtin_popcount(selmon->tagset[selmon->seltags] & tagmask_val) ==
                1 &&
            selmon->tagset[selmon->seltags] & (tagmask_val >> 1)) {
            selmon->seltags ^= 1; /* toggle sel tagset */
            selmon->tagset[selmon->seltags] =
                selmon->tagset[selmon->seltags ^ 1] << 1;
        } else {
            return;
        }
    }

    selmon->pertag->prevtag = selmon->pertag->current_tag;
    unsigned int new_tagset = selmon->tagset[selmon->seltags];

    if (new_tagset == ~0)
        selmon->pertag->current_tag = 0;
    else {
        for (i = 0; !(new_tagset & 1 << i); i++)
            ;
        selmon->pertag->current_tag = i + 1;
    }

    selmon->nmaster = PERTAG_NMASTER(selmon);
    selmon->mfact = PERTAG_MFACT(selmon);
    selmon->sellt = PERTAG_SELLT(selmon);
    selmon->lt[selmon->sellt] = PERTAG_LAYOUT(selmon);
    selmon->lt[selmon->sellt ^ 1] =
        selmon->pertag->ltidxs[PERTAG_CURRENT(selmon)][selmon->sellt ^ 1];

    if (selmon->showbar != PERTAG_SHOWBAR(selmon))
        togglebar(NULL);

    focus(NULL);
    arrange(selmon);
}

void viewtoleft(const Arg *arg) { viewscroll(arg, DirLeft); }

void moveleft(const Arg *arg) {
    tagtoleft(arg);
    viewtoleft(arg);
}

void shiftview(const Arg *arg) {
    Arg a;
    Client *c;
    unsigned visible = 0;
    int i = arg->i;
    int count = 0;
    int nextseltags, curseltags = selmon->tagset[selmon->seltags];

    do {
        if (i > 0) // left circular shift
            nextseltags = (curseltags << i) | (curseltags >> (numtags - 1 - i));
        else // right circular shift
            nextseltags =
                curseltags >> (-i) | (curseltags << (numtags - 1 + i));

        // Check if tag is visible
        for (c = selmon->clients; c && !visible; c = c->next)
            if (nextseltags & c->tags) {
                visible = 1;
                break;
            }
        i += arg->i;
    } while (!visible && ++count < 10);

    if (count < 10) {
        if (nextseltags & (SCRATCHPAD_MASK))
            nextseltags = nextseltags ^ (SCRATCHPAD_MASK);
        a.i = nextseltags;
        view(&a);
    }
}

void viewtoright(const Arg *arg) { viewscroll(arg, DirRight); }

void moveright(const Arg *arg) {
    tagtoright(arg);
    viewtoright(arg);
}

void overtoggle(const Arg *arg) {
    Client *c;
    c = selmon->sel;
    unsigned int tmptag;
    int showscratch = 0;

    if (!selmon->clients ||
        (selmon->clients == selmon->overlay && !selmon->overlay->next)) {
        if (PERTAG_CURRENT(selmon) == 0)
            lastview(NULL);
        return;
    }

    if (selmon->scratchvisible) {
        for (c = selmon->clients; c; c = c->next) {
            if (c->tags & SCRATCHPAD_MASK) {
                showscratch = 1;
                break;
            }
        }
        if (showscratch)
            togglescratchpad(NULL);
    }
    if (selmon->fullscreen)
        temp_fullscreen(NULL);
    if (PERTAG_CURRENT(selmon) == 0) {
        tmptag = selmon->pertag->prevtag;
        restoreallfloating(selmon);
        winview(NULL);
    } else {
        tmptag = PERTAG_CURRENT(selmon);
        saveallfloating(selmon);
        selmon->lt[selmon->sellt] = selmon->pertag->ltidxs[0][selmon->sellt] =
            (Layout *)&layouts[6];
        view(arg);
        if (selmon->lt[selmon->sellt] != (Layout *)&layouts[6])
            setlayout(&((Arg){.v = &layouts[6]}));
        focus(c);
    }
    selmon->pertag->prevtag = tmptag;
}

void lastview(const Arg *arg) {
    if (PERTAG_CURRENT(selmon) == selmon->pertag->prevtag)
        focus_last_client(NULL);
    else
        view(&((Arg){.ui = 1 << (selmon->pertag->prevtag - 1)}));
}

// overtoggle but with monocle layout
void fullovertoggle(const Arg *arg) {
    if (PERTAG_CURRENT(selmon) == 0) {
        winview(NULL);
    } else {
        selmon->lt[selmon->sellt] = selmon->pertag->ltidxs[0][selmon->sellt] =
            (Layout *)&layouts[3];
        view(arg);
    }
}

void winview(const Arg *arg) {
    Window win, win_r, win_p, *win_c;
    unsigned nc;
    int unused;
    Client *c;
    Arg a;

    if (&overviewlayout == tiling_layout_func(selmon)) {
        for (c = selmon->clients; c; c = c->next) {
            if (c == selmon->overlay)
                continue;
            if (c->isfloating)
                restorefloating(c);
        }
    }

    if (!XGetInputFocus(dpy, &win, &unused))
        return;
    while (XQueryTree(dpy, win, &win_r, &win_p, &win_c, &nc) && win_p != win_r)
        win = win_p;

    if (!(c = wintoclient(win)))
        return;

    a.ui = c->tags;
    if (c->tags == SCRATCHPAD_MASK) {
        view(&((Arg){.ui = 1 << (PERTAG_CURRENT(selmon) - 1)}));
    } else {
        view(&a);
    }
    focus(c);
}

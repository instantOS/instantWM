/* See LICENSE file for copyright and license details. */

#include <X11/Xlib.h>
#include <X11/Xresource.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "instantwm.h"
#include "xresources.h"

/* Extern declarations for arrays from config.h - defined in instantwm.c */
extern SchemePref schemehovertypes[];
extern SchemePref schemecolortypes[];
extern SchemePref schemewindowtypes[];
extern SchemePref schemetagtypes[];
extern SchemePref schemeclosetypes[];
extern const char *tagcolors[2][5][3];
extern const char *windowcolors[2][7][3];
extern const char *closebuttoncolors[2][3][3];
extern const char *bordercolors[];
extern const char *statusbarcolors[];
extern char tags[][16];
extern ResourcePref resources[];

/* Array sizes - need to be external or as macros/constants */
#define NUM_SCHEMEHOVERTYPES 2
#define NUM_SCHEMECOLORTYPES 3
#define NUM_SCHEMEWINDOWTYPES 7
#define NUM_SCHEMETAGTYPES 5
#define NUM_SCHEMECLOSETYPES 3
#define NUM_RESOURCES 11
#define MAX_TAGLEN 16

void list_xresources() {
    int i, u, q;
    for (i = 0; i < NUM_SCHEMEHOVERTYPES; i++) {
        for (q = 0; q < NUM_SCHEMECOLORTYPES; q++) {
            for (u = 0; u < NUM_SCHEMEWINDOWTYPES; u++) {
                char propname[100] = "";
                snprintf(propname, sizeof(propname), "%s.%s.win.%s",
                         schemehovertypes[i].name, schemewindowtypes[u].name,
                         schemecolortypes[q].name);
                printf("instantwm.%s\n", propname);
            }
            for (u = 0; u < NUM_SCHEMETAGTYPES; u++) {
                char propname[100] = "";
                snprintf(propname, sizeof(propname), "%s.%s.tag.%s",
                         schemehovertypes[i].name, schemetagtypes[u].name,
                         schemecolortypes[q].name);
                printf("instantwm.%s\n", propname);
            }
            for (u = 0; u < NUM_SCHEMECLOSETYPES; u++) {
                char propname[100] = "";
                snprintf(propname, sizeof(propname), "%s.%s.close.%s",
                         schemehovertypes[i].name, schemeclosetypes[u].name,
                         schemecolortypes[q].name);
                printf("instantwm.%s\n", propname);
            }
        }
    }
    printf(
        "normal.border\nfocus.tile.border\nfocus.float.border\nsnap.border\n");
    printf("status.fg\nstatus.bg\nstatus.detail\n");
}

void resource_load(XrmDatabase db, char *name, enum resource_type rtype,
                   void *dst) {
    char *sdst = NULL;
    int *idst = NULL;
    float *fdst = NULL;

    sdst = dst;
    idst = dst;
    fdst = dst;

    char fullname[256];
    char *type;
    XrmValue ret;

    snprintf(fullname, sizeof(fullname), "%s.%s", "instantwm", name);
    fullname[sizeof(fullname) - 1] = '\0';

    XrmGetResource(db, fullname, "*", &type, &ret);
    if (!(ret.addr == NULL || strncmp("String", type, 64))) {
        switch (rtype) {
        case STRING:
            strcpy(sdst, ret.addr);
            break;
        case INTEGER:
            *idst = strtoul(ret.addr, NULL, 10);
            break;
        case FLOAT:
            *fdst = strtof(ret.addr, NULL);
            break;
        }
    }
}

void load_xresources(void) {
    Display *display;
    char *resm;
    XrmDatabase db;
    ResourcePref *p;

    int i, u, q;

    display = XOpenDisplay(NULL);
    resm = XResourceManagerString(display);
    if (!resm)
        return;

    db = XrmGetStringDatabase(resm);

    for (i = 0; i < NUM_SCHEMEHOVERTYPES; i++) {
        for (q = 0; q < NUM_SCHEMECOLORTYPES; q++) {
            for (u = 0; u < NUM_SCHEMEWINDOWTYPES; u++) {
                char propname[100] = "";
                snprintf(propname, sizeof(propname), "%s.%s.win.%s",
                         schemehovertypes[i].name, schemewindowtypes[u].name,
                         schemecolortypes[q].name);

                // duplicate default value to avoid reading xresource into
                // multiple colors
                char *tmpstring = (char *)malloc((7 + 1) * sizeof(char));
                strcpy(tmpstring, windowcolors[schemehovertypes[i].type]
                                              [schemewindowtypes[u].type]
                                              [schemecolortypes[q].type]);

                /* Note: We can't modify windowcolors directly since it's const.
                   The original code had non-const arrays. This is a design
                   limitation that means load_xresources needs to stay in
                   instantwm.c for now */
                resource_load(db, propname, STRING, tmpstring);
                /* Would need: windowcolors[...][...][...] = tmpstring; */
            }

            for (u = 0; u < NUM_SCHEMETAGTYPES; u++) {
                char propname[100] = "";
                snprintf(propname, sizeof(propname), "%s.%s.tag.%s",
                         schemehovertypes[i].name, schemetagtypes[u].name,
                         schemecolortypes[q].name);

                char *tmpstring = (char *)malloc((7 + 1) * sizeof(char));

                strcpy(
                    tmpstring,
                    tagcolors[schemehovertypes[i].type][schemetagtypes[u].type]
                             [schemecolortypes[q].type]);

                resource_load(db, propname, STRING, tmpstring);
            }

            for (u = 0; u < NUM_SCHEMECLOSETYPES; u++) {
                char propname[100] = "";
                snprintf(propname, sizeof(propname), "%s.%s.close.%s",
                         schemehovertypes[i].name, schemeclosetypes[u].name,
                         schemecolortypes[q].name);

                char *tmpstring = (char *)malloc((7 + 1) * sizeof(char));
                strcpy(tmpstring, closebuttoncolors[schemehovertypes[i].type]
                                                   [schemeclosetypes[u].type]
                                                   [schemecolortypes[q].type]);

                resource_load(db, propname, STRING, tmpstring);
            }
        }
    }

    resource_load(db, "normal.border", STRING,
                  (void *)bordercolors[SchemeBorderNormal]);
    resource_load(db, "focus.tile.border", STRING,
                  (void *)bordercolors[SchemeBorderTileFocus]);
    resource_load(db, "focus.float.border", STRING,
                  (void *)bordercolors[SchemeBorderFloatFocus]);
    resource_load(db, "snap.border", STRING,
                  (void *)bordercolors[SchemeBorderSnap]);

    resource_load(db, "status.fg", STRING, (void *)statusbarcolors[ColFg]);
    resource_load(db, "status.bg", STRING, (void *)statusbarcolors[ColBg]);
    resource_load(db, "status.detail", STRING,
                  (void *)statusbarcolors[ColDetail]);

    for (p = resources; p < resources + NUM_RESOURCES; p++)
        resource_load(db, p->name, p->type, p->dst);

    XCloseDisplay(display);
}

void verifytagsxres(void) {
    for (int i = 0; i < 9; i++) {
        int len = strlen(tags[i]);
        if (len > MAX_TAGLEN - 1 || len == 0) {
            strcpy((char *)&tags[i], "Xres err");
        }
    }
}

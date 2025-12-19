/* See LICENSE file for copyright and license details. */

#include "commands.h"
#include "bar.h"
#include "client.h"
#include "focus.h"
#include "globals.h"
#include "instantwm.h"
#include "layouts.h"
#include "monitors.h"
#include "scratchpad.h"
#include "tags.h"
#include "toggles.h"
#include "util.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Implementation of xcommand */
int xcommand() {
    char command[256];
    char *fcursor; // walks through the command string as we go
    char *indicator = "c;:;";
    int i;
    int argnum;
    Arg arg;

    // Get root name property
    int got_command = gettextprop(root, XA_WM_NAME, command, sizeof(command));
    if (!got_command || !startswith(command, indicator)) {
        return 0; // no command for us passed, get out
    }
    fcursor =
        command + strlen(indicator); // got command for us, strip indicator

    // Check if a command was found, and if so handle it
    for (i = 0; i < commands_len; i++) {
        if (!startswith(fcursor, commands[i].cmd)) {
            continue;
        }

        fcursor += strlen(commands[i].cmd);
        // no args
        if (!strlen(fcursor)) {
            arg = commands[i].arg;
        } else {
            if (fcursor[0] != ';') {
                // longer command staring with the same letters?
                fcursor -= strlen(commands[i].cmd);
                continue;
            }
            fcursor++;
            switch (commands[i].type) {
            case CmdArgNone:
                arg = commands[i].arg;
                break;
            case CmdArgToggle:
                argnum = atoi(fcursor);
                if (argnum != 0 && fcursor[0] != '0') {
                    arg = ((Arg){.ui = atoi(fcursor)});
                } else {
                    arg = commands[i].arg;
                }
                break;
            case CmdArgTag:
                argnum = atoi(fcursor);
                if (argnum != 0 && fcursor[0] != '0') {
                    arg = ((Arg){.ui = (1 << (atoi(fcursor) - 1))});
                } else {
                    arg = commands[i].arg;
                }
                break;
            case CmdArgString:
                arg = ((Arg){.v = fcursor});
                break;
            case CmdArgInt:
                if (fcursor[0] != '\0') {
                    arg = ((Arg){.i = atoi(fcursor)});
                } else {
                    arg = commands[i].arg;
                }
                break;
            }
        }
        commands[i].func(&(arg));
        break;
    }
    return 1;
}

void setspecialnext(const Arg *arg) { specialnext = arg->ui; }

void commandprefix(const Arg *arg) {
    tagprefix = arg->ui;
    drawbar(selmon);
}

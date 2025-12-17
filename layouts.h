#ifndef LAYOUTS_H
#define LAYOUTS_H

#include "instantwm.h"

void bstack(Monitor *m);
void bstackhoriz(Monitor *m);
void deck(Monitor *m);
void grid(Monitor *m);
void monocle(Monitor *m);
void overviewlayout(Monitor *m);
void tcl(Monitor *m);
void tile(Monitor *m);
void floatl(Monitor *m);
<<<<<<< HEAD

static inline void (*tiling_layout_func(Monitor *m))(Monitor *) {
    return m->lt[m->sellt]->arrange;
}

=======
>>>>>>> origin/main
static inline Client *findVisibleClient(Client *c) {
    Client *client = NULL;
    for (client = c; client; client = client->next) {
        if (ISVISIBLE(client))
            return client;
    }
    return NULL;
}
#endif

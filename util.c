/* See LICENSE file for copyright and license details. */
#include <signal.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "globals.h"
#include "instantwm.h"
#include "util.h"

void *ecalloc(size_t nmemb, size_t size) {
    void *p;

    if (!(p = calloc(nmemb, size))) {
        die("calloc:");
    }
    return p;
}

void die(const char *fmt, ...) {
    va_list ap;

    va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    va_end(ap);

    if (fmt[0] && fmt[strlen(fmt) - 1] == ':') {
        fputc(' ', stderr);
        perror(NULL);
    } else {
        fputc('\n', stderr);
    }

    exit(1);
}

int startswith(const char *a, const char *b) {
    char *checker = NULL;

    checker = strstr(a, b);
    if (checker == a) {
        return 1;
    }
    return 0;
}

void spawn(const Arg *arg) {
    struct sigaction sa;
    if (arg->v == instantmenucmd) {
        instantmenumon[0] = '0' + selmon->num;
    }
    if (fork() == 0) {
        if (dpy) {
            close(ConnectionNumber(dpy));
        }
        setsid();
        sigemptyset(&sa.sa_mask);
        sa.sa_flags = 0;
        sa.sa_handler = SIG_DFL;
        sigaction(SIGCHLD, &sa, NULL);
        execvp(((char **)arg->v)[0], (char **)arg->v);
        die("instantwm: execvp '%s' failed:", ((char **)arg->v)[0]);
    }
}

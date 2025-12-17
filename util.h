/* See LICENSE file for copyright and license details. */
#ifndef UTIL_H
#define UTIL_H

#define MAX(A, B) ((A) > (B) ? (A) : (B))
#define MIN(A, B) ((A) < (B) ? (A) : (B))
#define BETWEEN(X, A, B) ((A) <= (X) && (X) <= (B))

void die(const char *fmt, ...);
void *ecalloc(size_t nmemb, size_t size);
<<<<<<< HEAD
int startswith(const char *a, const char *b);
=======
>>>>>>> origin/main
#endif

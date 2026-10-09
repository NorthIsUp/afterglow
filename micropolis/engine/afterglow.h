/* afterglow: not part of the Micropolis release. Included by micropolis.h so
 * every assert and fatal path in the engine reaches the setjmp boundary in
 * ../afterglow_micropolis.cpp instead of ending the screensaver's process.
 *
 * Same licence as the engine: GPL-3.0-or-later, with EA's additional terms
 * (see micropolis.h).
 */
#pragma once

#include <assert.h>
#include <stdio.h>

#undef assert
#define assert(e) ((e) ? (void)0 : afterglow_fatal(#e, __FILE__, __LINE__))

[[noreturn]] void afterglow_fatal(const char *what, const char *file, int line);

/* fopen, except a name of the form "mem:<n>" opens bundled city <n>. */
FILE *afterglow_fopen(const char *name, const char *mode);

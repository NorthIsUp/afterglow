// The autopilot's shared state: a walkability grid over the level (ap_nav.c)
// and the player that plays on it (autopilot.c).
//
// Copyright (C) 2026 Adam Hitchcock. GPL-2.0-or-later, like the engine it
// links against; see doomgeneric/LICENSE.

#ifndef AFTERGLOW_AUTOPILOT_H
#define AFTERGLOW_AUTOPILOT_H

#include "d_player.h"
#include "d_ticcmd.h"
#include "m_fixed.h"
#include "r_defs.h"

#define NAV_NONE (-1)
#define NAV_FAR 0x3fffffff
// nav_step's *use when the way opens by itself: a lift or door on the move.
#define NAV_WAIT (-2)

// 1 to play in god mode: then damaging floors cost little and health is not
// sought.
extern int ap_god;
// The stuck fallback's jumps so far, and a seed for the autopilot's dice: for
// tools/doom-bench.
extern int ap_hops;
extern uint32_t ap_seed;

extern int nav_w, nav_h, nav_cs;

// Rebuild the grid for the level just loaded. Allocates only when a level is
// bigger than any before it.
void nav_build(void);
int nav_cell(fixed_t x, fixed_t y);
void nav_center(int c, fixed_t *x, fixed_t *y);
// Where in cell c the player stands clear of walls: its centre, or near it.
void nav_stand(int c, fixed_t *x, fixed_t *y);
int nav_sector(int c);
// The player can step from one floor and ceiling to the other as they stand.
int nav_can_step(fixed_t from_floor, fixed_t from_ceil, fixed_t to_floor, fixed_t to_ceil);

// Dijkstra from `start` over every cell the player can reach now, given its
// keys: doors it can open and lifts it can call count as open.
void nav_flood(int start, player_t *p);
int nav_dist(int c);
// The cells from the flood's start to `goal`, at most `max`; 0 if unreachable.
int nav_path(int goal, int *out, int max);
// Stepping from cell a into the next path cell b (a neighbour, or a
// teleporter's destination): -1 if it is closed to the player, else 0, with
// *use the line to activate first (-1 none, NAV_WAIT to stand and wait), *via the first line crossed into
// another sector (or -1), and *tx, *ty the next cell's centre.
int nav_step(int a, int b, player_t *p, int *use, int *via, fixed_t *tx, fixed_t *ty);
// Remember which cells the last flood reached (returns how many); after
// another flood, the cells it reaches that the snapshot did not, and in *lost
// the reverse.
int nav_snapshot(void);
int nav_gain(int *lost);
int nav_was_reached(int c);
// Re-judge where the player can stand next to sectors whose heights changed
// since the last call.
void nav_moved(void);
// How many of cell c's neighbours the player can step to: 0 for a closet.
int nav_exits(int c, player_t *p);
// Make a cell expensive for a while, after the player failed to cross it.
void nav_penalize(int c);
void nav_decay(void);

// Line specials the autopilot reasons about.
int ap_door_key(int special);  // -1 not a manual door, 0 no key, 1..3 blue/yellow/red
int ap_lock_key(int special);  // key a remote locked-door switch needs, 0 none
int ap_has_key(player_t *p, int key);
int ap_is_lift(int special);
int ap_is_exit(int special);   // 1 normal exit, 2 secret exit
int ap_is_use(int special);    // a switch or door the player presses
int ap_is_teleport(int special);

// What switch, walk-over and gun lines do (ap_effect.c): a hypothesis of
// sector heights over the real ones, which the nav grid searches through.
enum { FX_WALK = 1, FX_USE, FX_SHOOT };
void fx_build(void);
int fx_trigger(int special);  // FX_*, or 0 for a line that moves nothing the grid sees
int fx_repeat(int special);
int fx_shuts(int special);    // it closes a door or brings a ceiling down
// Lay line li's effect over the hypothesis; the number of sectors it changes.
int fx_push(int li);
void fx_reset(void);
fixed_t fx_floor(int s);
fixed_t fx_ceil(int s);
fixed_t fx_floor_hi(int s);  // a lift's top: it rides between this and fx_floor
int fx_ranged(int s);
int fx_active(int s);
int fx_moves(int s);  // some line can move this sector

#endif

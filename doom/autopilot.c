// The autopilot: plays a level to its exit, fighting what it sees on the way.
//
// Copyright (C) 2026 Adam Hitchcock. GPL-2.0-or-later, like the engine it
// links against; see doomgeneric/LICENSE.
//
// Once a second, and whenever its errand is done or the way is shut, it
// floods the grid (ap_nav.c) from where the player stands and picks one
// errand by path cost, in this order:
//
//   health when low (god off), then wanted pickups close by,
//   the exit when it is reachable (after reachable secrets, early on),
//   a line whose effect opens the way to the exit, keys, a line that opens
//   the way to a key, secrets, lines that open new ground, unseen ground,
//   the monsters still alive, and last the lines that seem to do nothing.
//
// What a line does comes from ap_effect.c: its effect is laid over the level
// and the grid flooded again from where the player would set it off.
//
// It walks the path, pressing doors and lift switches that stand in the way,
// strafing along it while it turns to shoot the nearest monster it can see.
// A cell it keeps failing to cross gets expensive; an errand that fails three
// times or outlasts its walk by 25 s is dropped; 15 s rooted to one spot jumps
// to a random item. docs/savers/doom.md has the whole story.

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "autopilot.h"
#include "d_event.h"
#include "doomstat.h"
#include "info.h"
#include "p_local.h"
#include "r_main.h"
#include "r_state.h"
#include "tables.h"

#define TIC 35
#define PATH_MAX_CELLS 4096
#define BANNED_THINGS 64
#define BANNED 3
#define SECRET_TICS (90 * TIC)
#define ERRAND_TICS (25 * TIC)
#define REPLAN_TICS TIC
#define USE_REACH (60 * FRACUNIT)
#define FIGHT_RANGE (1400 * FRACUNIT)

int ap_god = 1;
uint32_t ap_seed;
int ap_hops;

// K_OPENER: a line whose effect makes the exit or a key reachable. K_SWITCH:
// one that opens up new ground. K_TRIGGER: one the model sees doing nothing
// useful, tried only when there is nothing left to explore.
enum kind { K_NONE, K_HEALTH, K_ITEM, K_EXIT, K_KEY, K_OPENER, K_SECRET, K_SWITCH, K_EXPLORE, K_HUNT, K_TRIGGER };
enum act { A_TOUCH, A_USE, A_CROSS, A_SHOOT };

static struct {
    enum kind kind;
    enum act act;
    int cell, line, sector;
    mobj_t *thing;
    fixed_t x, y;  // where to stand, or what to touch
    int since, sub, budget;
    int side;  // for a walk-over line, the side the player set out from
} job;

static int path[PATH_MAX_CELLS], plen, pi;
static int level_ep = -1, level_map = -1, last_time;
static mobj_t *last_mo;
static int ride_until;
static int replan_at, noprog, best_d, moved_at, hopped_at, jiggle, use_wait, waited;
static fixed_t anchor_x, anchor_y;
static mobj_t *banned_things[BANNED_THINGS];
static int nbanned;
static unsigned char *line_ban, *sec_ban, *seen, *lflags;
static int *lgain;
static int line_cap, sec_cap, seen_cap;
static uint32_t rng = 1;
// Set while it faces a line to press it, so a monster does not turn it away;
// and while it shoots one, which takes a gun that fires bullets.
static int pressing, shooting;

static uint32_t rnd(void) {
    rng ^= rng << 13;
    rng ^= rng >> 17;
    rng ^= rng << 5;
    return rng;
}

static void *grow(void *p, size_t n) {
    void *q = realloc(p, n);
    if (!q) abort();
    return q;
}

static int is_mobj(thinker_t *t) { return t->function.acp1 == (actionf_p1)P_MobjThinker; }

static void level_start(void) {
    nav_build();
    if (numlines > line_cap) {
        line_cap = numlines;
        line_ban = grow(line_ban, line_cap);
        lflags = grow(lflags, line_cap);
        lgain = grow(lgain, line_cap * sizeof *lgain);
    }
    if (numsectors > sec_cap) sec_ban = grow(sec_ban, sec_cap = numsectors);
    if (nav_w * nav_h > seen_cap) seen = grow(seen, seen_cap = nav_w * nav_h);
    memset(line_ban, 0, numlines);
    memset(lflags, 0, numlines);
    memset(sec_ban, 0, numsectors);
    memset(seen, 0, nav_w * nav_h);
    nbanned = plen = pi = 0;
    job.kind = K_NONE;
    replan_at = noprog = moved_at = hopped_at = jiggle = use_wait = waited = ride_until = 0;
    best_d = NAV_FAR;
}

// ---- what is worth fetching ----

static int thing_banned(mobj_t *m) {
    for (int i = 0; i < nbanned; i++)
        if (banned_things[i] == m) return 1;
    return 0;
}

static void ban_thing(mobj_t *m) {
    if (nbanned < BANNED_THINGS) banned_things[nbanned++] = m;
}

static int key_of(mobj_t *m) {
    switch (m->sprite) {
    case SPR_BKEY: case SPR_BSKU: return 1;
    case SPR_YKEY: case SPR_YSKU: return 2;
    case SPR_RKEY: case SPR_RSKU: return 3;
    default: return 0;
    }
}

static int is_health(mobj_t *m) {
    return m->sprite == SPR_STIM || m->sprite == SPR_MEDI || m->sprite == SPR_SOUL || m->sprite == SPR_MEGA ||
           m->sprite == SPR_BON1;
}

static int ammo_wanted(player_t *p, ammotype_t a) { return p->ammo[a] < p->maxammo[a]; }

// Whether touching it would take it: Doom leaves full-up pickups lying there.
static int wanted(player_t *p, mobj_t *m) {
    if (!(m->flags & MF_SPECIAL) || thing_banned(m)) return 0;
    switch (m->sprite) {
    case SPR_BKEY: case SPR_BSKU: case SPR_YKEY: case SPR_YSKU: case SPR_RKEY: case SPR_RSKU:
        return !ap_has_key(p, key_of(m));
    case SPR_STIM: case SPR_MEDI: return p->health < 100;
    case SPR_BON1: case SPR_SOUL: return p->health < 200;
    case SPR_ARM1: return p->armorpoints < 100;
    case SPR_ARM2: case SPR_BON2: return p->armorpoints < 200;
    case SPR_CLIP: case SPR_AMMO: return ammo_wanted(p, am_clip);
    case SPR_SHEL: case SPR_SBOX: return ammo_wanted(p, am_shell);
    case SPR_ROCK: case SPR_BROK: return ammo_wanted(p, am_misl);
    case SPR_CELL: case SPR_CELP: return ammo_wanted(p, am_cell);
    case SPR_SHOT: return !p->weaponowned[wp_shotgun] || ammo_wanted(p, am_shell);
    case SPR_SGN2: return !p->weaponowned[wp_supershotgun] || ammo_wanted(p, am_shell);
    case SPR_MGUN: return !p->weaponowned[wp_chaingun] || ammo_wanted(p, am_clip);
    case SPR_LAUN: return !p->weaponowned[wp_missile] || ammo_wanted(p, am_misl);
    case SPR_PLAS: return !p->weaponowned[wp_plasma] || ammo_wanted(p, am_cell);
    case SPR_BFUG: return !p->weaponowned[wp_bfg] || ammo_wanted(p, am_cell);
    case SPR_CSAW: return !p->weaponowned[wp_chainsaw];
    case SPR_BPAK: return !p->backpack;
    default: return 1;
    }
}

// Without god mode, how far it will go for a pickup it does not see: far
// for a first real gun, armour or ammo when short, close by otherwise.
static int needed(player_t *p, mobj_t *m) {
    int armed = p->weaponowned[wp_shotgun] || p->weaponowned[wp_chaingun] || p->weaponowned[wp_supershotgun];
    switch (m->sprite) {
    case SPR_SHOT: case SPR_SGN2: case SPR_MGUN: return armed ? 600 : 3000;
    case SPR_ARM1: case SPR_ARM2: return p->armorpoints < 50 ? 1500 : 600;
    case SPR_CLIP: case SPR_AMMO: return p->ammo[am_clip] < 50 ? 1500 : 480;
    case SPR_SHEL: case SPR_SBOX: return p->weaponowned[wp_shotgun] && p->ammo[am_shell] < 12 ? 1500 : 480;
    default: return 480;
    }
}

// The cell to walk to for pickup m: its own, or for one on a ledge too high
// to climb, a reachable cell beside it. Doom hands over a pickup up to 56
// units above the player's feet once their boxes touch.
static int pickup_cell(mobj_t *m) {
    int c = nav_cell(m->x, m->y);
    if (nav_dist(c) < NAV_FAR) return c;
    static const signed char off[8][2] = {{32, 0}, {-32, 0}, {0, 32}, {0, -32}, {24, 24}, {-24, 24}, {24, -24}, {-24, -24}};
    int best = c;
    for (int i = 0; i < 8; i++) {
        int n = nav_cell(m->x + off[i][0] * FRACUNIT, m->y + off[i][1] * FRACUNIT);
        int s = nav_sector(n);
        if (s < 0 || m->z - sectors[s].floorheight >= 56 * FRACUNIT) continue;
        if (nav_dist(n) < nav_dist(best)) best = n;
    }
    return best;
}

// ---- choosing an errand ----

typedef struct {
    enum kind kind;
    enum act act;
    int cell, line, sector, d;
    mobj_t *thing;
    fixed_t x, y;
} cand_t;

static void offer(cand_t *best, enum kind k, enum act a, int cell, fixed_t x, fixed_t y) {
    int d = nav_dist(cell);
    if (d >= best->d) return;
    *best = (cand_t){k, a, cell, -1, -1, d, NULL, x, y};
}

static line_t *sight_line;
static int sight_ok;

static boolean sight_cb(intercept_t *in) {
    line_t *l = in->d.line;
    if (l == sight_line) return false;
    if (!l->backsector || (l->flags & ML_BLOCKING)) return sight_ok = 0;
    P_LineOpening(l);
    if (openrange < 16 * FRACUNIT) return sight_ok = 0;
    return true;
}

// A bullet from (x, y) to the middle of line l crosses nothing before it.
static int shot_clear(fixed_t x, fixed_t y, line_t *l) {
    sight_line = l;
    sight_ok = 1;
    P_PathTraverse(x, y, l->v1->x + l->dx / 2, l->v1->y + l->dy / 2, PT_ADDLINES, sight_cb);
    return sight_ok;
}

// Whether the player can walk over two-sided line l from its side `side`:
// a walk-over line fires only for a move Doom lets through, so one on the lip
// of a ledge triggers from the top, never from below.
static int crossable(line_t *l, int side) {
    if (!l->backsector || (l->flags & ML_BLOCKING)) return 0;
    sector_t *from = side ? l->backsector : l->frontsector, *to = side ? l->frontsector : l->backsector;
    fixed_t top = from->ceilingheight < to->ceilingheight ? from->ceilingheight : to->ceilingheight;
    fixed_t bot = from->floorheight > to->floorheight ? from->floorheight : to->floorheight;
    return to->floorheight - from->floorheight <= 24 * FRACUNIT && top - bot >= 56 * FRACUNIT;
}

// The best place to stand to press line l (side 0, its front), to walk over
// it (either side), or to shoot it (either side, out to 384 units, in clear
// view of its middle); best->d stays NAV_FAR if none is reachable.
static void offer_line(cand_t *best, enum kind k, int li, enum act act) {
    line_t *l = &lines[li];
    fixed_t len = P_AproxDistance(l->dx, l->dy);
    if (len < FRACUNIT) return;
    fixed_t ux = FixedDiv(l->dy, len), uy = -FixedDiv(l->dx, len);
    static const int frac[3] = {2, 1, 3}, reach[4] = {96, 192, 288, 384};
    if (act == A_SHOOT) {
        for (int side = 0; side < 2; side++)
            for (int i = 0; i < 4; i++) {
                int r = side ? -reach[i] : reach[i];
                fixed_t px = l->v1->x + l->dx / 2 + ux * r, py = l->v1->y + l->dy / 2 + uy * r;
                int c = nav_cell(px, py), before = best->d;
                if (nav_dist(c) >= before || !shot_clear(px, py, l)) continue;
                offer(best, k, act, c, px, py);
                if (best->d < before) best->line = li;
            }
        return;
    }
    // Three standoffs: a switch at the back of a closet too shallow to stand
    // in is pressed from the room outside it.
    for (int side = 0; side < (act == A_CROSS ? 2 : 1); side++)
        for (int i = 0; i < 9 && (act != A_CROSS || crossable(l, side)); i++) {
            int out = i < 3 ? 28 : i < 6 ? 44 : 60;
            fixed_t px = l->v1->x + l->dx / 4 * frac[i % 3];
            fixed_t py = l->v1->y + l->dy / 4 * frac[i % 3];
            if (side) px -= ux * out, py -= uy * out;
            else px += ux * out, py += uy * out;
            int c = nav_cell(px, py);
            int before = best->d;
            offer(best, k, act, c, px, py);
            if (best->d < before) best->line = li;
        }
}

static int tier(enum kind k) {
    switch (k) {
    case K_HEALTH: return 0;
    case K_ITEM: return 1;
    case K_EXIT: return 2;
    case K_KEY: return 3;
    case K_OPENER: return 4;
    case K_SECRET: return 5;
    case K_SWITCH: return 6;
    case K_EXPLORE: return 7;
    case K_HUNT: return 8;
    case K_TRIGGER: return 9;
    default: return 10;
    }
}

// ---- what a line would open ----

// How the player sets line li off, or -1 when it is not worth a trip: doors
// it opens by hand are the grid's business, and a gun line needs a gun that
// fires bullets.
static int line_act(player_t *p, int li) {
    int s = lines[li].special;
    if (ap_is_exit(s) || ap_door_key(s) >= 0) return -1;
    switch (fx_trigger(s)) {
    case FX_USE: return ap_has_key(p, ap_lock_key(s)) ? A_USE : -1;
    case FX_WALK: return A_CROSS;
    case FX_SHOOT:
        return p->ammo[am_clip] > 0 || (p->weaponowned[wp_shotgun] && p->ammo[am_shell] > 0) ? A_SHOOT : -1;
    default: return -1;
    }
}

enum { L_DONE = 1, L_EXIT = 2, L_KEY = 4, L_HARM = 8 };
#define LINE_CANDS 128
#define EVALS_PER_PLAN 4
#define GAIN_MIN 4
#define SCORE_TICS (10 * TIC)

static cand_t lcand[LINE_CANDS];
static int nlcand;
static uint32_t world_sig;
static int scored_at;
int ap_evals;
double ap_eval_ms, ap_eval_max_ms;

static void add_line_cand(const cand_t *c) {
    if (nlcand < LINE_CANDS) {
        lcand[nlcand++] = *c;
        return;
    }
    int far = 0;
    for (int i = 1; i < LINE_CANDS; i++)
        if (lcand[i].d > lcand[far].d) far = i;
    if (c->d < lcand[far].d) lcand[far] = *c;
}

// Changes when a line fires for good or a key is taken. Heights are left
// out: lifts, doors and crushers move them all the time.
static uint32_t signature(player_t *p) {
    uint32_t h = 2166136261u;
    for (int i = 0; i < numlines; i++) h += lines[i].special != 0;
    for (int k = 1; k <= 3; k++) h = (h ^ (uint32_t)ap_has_key(p, k)) * 16777619u;
    return h;
}

static int exit_reachable(void) {
    for (int i = 0; i < numlines; i++) {
        int s = lines[i].special;
        if (ap_is_exit(s) != 1) continue;
        cand_t e = {K_NONE, A_TOUCH, -1, -1, -1, NAV_FAR, NULL, 0, 0};
        offer_line(&e, K_EXIT, i, s == 52 ? A_CROSS : A_USE);
        if (e.d < NAV_FAR) return 1;
    }
    return 0;
}

static int key_opened(player_t *p) {
    for (thinker_t *t = thinkercap.next; t != &thinkercap; t = t->next) {
        if (!is_mobj(t) || !key_of((mobj_t *)t)) continue;
        mobj_t *m = (mobj_t *)t;
        int c = pickup_cell(m);
        if (wanted(p, m) && nav_dist(c) < NAV_FAR && !nav_was_reached(c)) return 1;
    }
    return 0;
}

// Score the candidate lines against the flood just made from `here`: lay
// each one's effect over the level, flood again from where the player would
// stand to set it off, and see what it opens. A few new lines a second;
// scores hold for 10 s, or until a line fires for good or a key is taken.
// Leaves the flood as it found it.
static void score_lines(player_t *p, int here) {
    if (!nlcand) return;
    nav_snapshot();
    uint32_t sig = signature(p);
    if (sig != world_sig || leveltime - scored_at > SCORE_TICS) {
        world_sig = sig;
        scored_at = leveltime;
        memset(lflags, 0, numlines);
    }
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    int evals = 0;
    for (int i = 0; i < nlcand && evals < EVALS_PER_PLAN; i++) {
        int li = lcand[i].line;
        if (lflags[li] & L_DONE) continue;
        lflags[li] = L_DONE;
        lgain[li] = 0;
        fx_reset();
        if (!fx_push(li)) continue;
        evals++;
        nav_flood(lcand[i].cell, p);
        int lost, gain = nav_gain(&lost);
        lgain[li] = gain;
        if (exit_reachable()) lflags[li] |= L_EXIT;
        if (key_opened(p)) lflags[li] |= L_KEY;
        // A line that shuts something and opens nothing is left alone. The
        // flood starts where the line is, so cells behind a one-way drop
        // count as lost for any line: that cost is the trip's, not the line's.
        if (fx_shuts(lines[li].special) && gain < GAIN_MIN && lost >= 8) lflags[li] |= L_HARM;
#ifdef AP_TRACE
        printf("    line %d sp %d tag %d: gain %d lost %d%s%s\n", li, lines[li].special, lines[li].tag, gain, lost,
               lflags[li] & L_EXIT ? " EXIT" : "", lflags[li] & L_KEY ? " KEY" : "");
#endif
    }
    fx_reset();
    if (!evals) return;
    nav_flood(here, p);
    clock_gettime(CLOCK_MONOTONIC, &t1);
    double ms = (t1.tv_sec - t0.tv_sec) * 1e3 + (t1.tv_nsec - t0.tv_nsec) / 1e6;
    ap_evals += evals;
    ap_eval_ms += ms;
    if (ms > ap_eval_max_ms) ap_eval_max_ms = ms;
}

// The candidate lines by what scoring found: one that opens the way to the
// exit, or to a key, one that opens new ground, and the rest.
static void pick_lines(cand_t *to_exit, cand_t *to_key, cand_t *opens, cand_t *rest) {
    for (int i = 0; i < nlcand; i++) {
        cand_t c = lcand[i];
        int f = lflags[c.line];
        if (f & L_HARM) continue;
        cand_t *b = (f & L_EXIT) ? to_exit : (f & L_KEY) ? to_key : lgain[c.line] >= GAIN_MIN ? opens : rest;
        if (!(f & L_DONE)) b = rest;
        c.kind = b == to_exit || b == to_key ? K_OPENER : b == opens ? K_SWITCH : K_TRIGGER;
        if (c.d < b->d) *b = c;
    }
}

static void set_job(const cand_t *c, int sub) {
    if (c->kind != job.kind || c->cell != job.cell || c->line != job.line) {
        job.since = leveltime;
        use_wait = 0;
        // A far errand gets the time its walk takes, at a fighting pace.
        job.budget = (c->kind == K_SECRET ? ERRAND_TICS / 2 : ERRAND_TICS) + (c->d < NAV_FAR ? c->d / 8 : 0);
        noprog = 0;
        best_d = NAV_FAR;
    }
    job.kind = c->kind;
    job.act = c->act;
    job.cell = c->cell;
    job.line = c->line;
    job.sector = c->sector;
    job.thing = c->thing;
    job.x = c->x;
    job.y = c->y;
    job.sub = sub;
    job.side = job.act == A_CROSS && job.line >= 0 ? P_PointOnLineSide(job.x, job.y, &lines[job.line]) : -1;
    plen = job.kind == K_NONE ? 0 : nav_path(job.cell, path, PATH_MAX_CELLS);
    pi = 0;
    noprog = 0;
    best_d = NAV_FAR;
    replan_at = leveltime + REPLAN_TICS;
}

static void choose(player_t *p) {
    mobj_t *me = p->mo;
    int here = nav_cell(me->x, me->y);
    nav_moved();
    nav_flood(here, p);
    cand_t c = {K_NONE, A_TOUCH, -1, -1, -1, NAV_FAR, NULL, 0, 0};

    // Fetch: health when it matters, otherwise what is in sight and close.
    for (thinker_t *t = thinkercap.next; t != &thinkercap; t = t->next) {
        if (!is_mobj(t)) continue;
        mobj_t *m = (mobj_t *)t;
        if (!wanted(p, m)) continue;
        int cell = pickup_cell(m), d = nav_dist(cell);
        if (!ap_god && is_health(m) && d < (p->health < 50 ? 3000 : p->health < 80 ? 800 : 0)) {
            if (c.kind != K_HEALTH || d < c.d) c = (cand_t){K_HEALTH, A_TOUCH, cell, -1, -1, d, m, m->x, m->y};
        } else if (c.kind != K_HEALTH && d < (ap_god ? 480 : needed(p, m)) && d < c.d && (!ap_god || P_CheckSight(me, m))) {
            c = (cand_t){K_ITEM, A_TOUCH, cell, -1, -1, d, m, m->x, m->y};
        }
    }

    cand_t exit = {K_NONE, A_TOUCH, -1, -1, -1, NAV_FAR, NULL, 0, 0};
    cand_t secret = exit, key = exit, explore = exit, hunt = exit;
    cand_t to_exit = exit, to_key = exit, opens = exit, rest = exit;
    nlcand = 0;
    for (int i = 0; i < numlines; i++) {
        int s = lines[i].special;
        if (!s || line_ban[i] >= BANNED) continue;
        int ex = ap_is_exit(s), act;
        if (ex) {
            cand_t e = exit;
            e.d = NAV_FAR;
            offer_line(&e, K_EXIT, i, s == 52 || s == 124 ? A_CROSS : A_USE);
            if (ex == 2) e.d += 2000;  // the secret exit only when it is far nearer
            if (e.d < exit.d) exit = e;
        } else if ((act = line_act(p, i)) >= 0) {
            cand_t l = exit;
            offer_line(&l, K_TRIGGER, i, (enum act)act);
            if (l.d < NAV_FAR) add_line_cand(&l);
        }
    }
    if (leveltime < SECRET_TICS) {
        for (int cell = 0; cell < nav_w * nav_h; cell++) {
            int s = nav_sector(cell);
            if (s >= 0 && sectors[s].special == 9 && sec_ban[s] < BANNED && nav_dist(cell) < 1500) {
                fixed_t x, y;
                nav_center(cell, &x, &y);
                int before = secret.d;
                offer(&secret, K_SECRET, A_TOUCH, cell, x, y);
                if (secret.d < before) secret.sector = s;
            }
        }
    }
    for (thinker_t *t = thinkercap.next; t != &thinkercap; t = t->next) {
        if (!is_mobj(t) || !key_of((mobj_t *)t)) continue;
        mobj_t *m = (mobj_t *)t;
        if (!wanted(p, m)) continue;
        int before = key.d;
        offer(&key, K_KEY, A_TOUCH, pickup_cell(m), m->x, m->y);
        if (key.d < before) key.thing = m;
    }
    for (int cell = 0; cell < nav_w * nav_h; cell++) {
        if (seen[cell] || nav_dist(cell) == NAV_FAR) continue;
        fixed_t x, y;
        nav_center(cell, &x, &y);
        offer(&explore, K_EXPLORE, A_TOUCH, cell, x, y);
    }

    // Nothing left to open or see: go after the monsters still alive. A boss
    // map's way out opens when its bosses die.
    if (explore.d == NAV_FAR) {
        for (thinker_t *t = thinkercap.next; t != &thinkercap; t = t->next) {
            if (!is_mobj(t)) continue;
            mobj_t *m = (mobj_t *)t;
            if (!(m->flags & MF_COUNTKILL) || m->health <= 0 || thing_banned(m)) continue;
            int before = hunt.d;
            offer(&hunt, K_HUNT, A_TOUCH, nav_cell(m->x, m->y), m->x, m->y);
            if (hunt.d < before) hunt.thing = m;
        }
    }
    if (c.kind == K_NONE && exit.d == NAV_FAR) {
        score_lines(p, here);
        pick_lines(&to_exit, &to_key, &opens, &rest);
    }
    if (c.kind == K_NONE) {
        if (exit.d < NAV_FAR) c = secret.d < NAV_FAR ? secret : exit;
        else if (to_exit.d < NAV_FAR) c = to_exit;
        else if (key.d < NAV_FAR) c = key;
        else if (to_key.d < NAV_FAR) c = to_key;
        else if (secret.d < NAV_FAR) c = secret;
        else if (opens.d < NAV_FAR) c = opens;
        else if (explore.d < NAV_FAR) c = explore;
        else if (hunt.d < NAV_FAR) c = hunt;
        else c = rest;
    }
    // Hold an errand that is still reachable unless something of a higher
    // order turns up, or the same order much nearer: flipping between
    // targets each second reads as dithering on screen.
    int held = nav_dist(job.cell);
    if (job.kind != K_NONE && held < NAV_FAR &&
        (job.kind != c.kind ? tier(c.kind) >= tier(job.kind)
                            : c.cell != job.cell && c.d * 10 > held * 6 && job.kind != K_ITEM && job.kind != K_HEALTH)) {
        plen = nav_path(job.cell, path, PATH_MAX_CELLS);
        pi = 0;
        replan_at = leveltime + REPLAN_TICS;
        return;
    }
    set_job(&c, 0);
#ifdef AP_TRACE
    printf("  t=%5.1f at %5d,%5d  job %d act %d line %d(sp %d) sec %d d=%d plen=%d exit=%d key=%d open=%d/%d/%d\n",
           leveltime / 35.0, me->x >> FRACBITS, me->y >> FRACBITS, job.kind, job.act, job.line,
           job.line >= 0 ? lines[job.line].special : 0, job.sector, c.d, plen, exit.d, key.d, to_exit.d, opens.d, rest.d);
#endif
}

static void drop_job(void) {
#ifdef AP_TRACE
    printf("  t=%5.1f drop job %d line %d\n", leveltime / 35.0, job.kind, job.line);
#endif
    if (job.thing) ban_thing(job.thing);
    if (job.line >= 0) line_ban[job.line] = BANNED;
    if (job.sector >= 0) sec_ban[job.sector] = BANNED;
    if (job.kind == K_EXPLORE && job.cell >= 0) seen[job.cell] = 1;
    job.kind = K_NONE;
    replan_at = 0;
}

// ---- steering ----

static int clear_ok;
static fixed_t clear_z;

static boolean clear_cb(intercept_t *in) {
    line_t *l = in->d.line;
    if (!l->backsector || (l->flags & ML_BLOCKING)) return clear_ok = 0;
    P_LineOpening(l);
    if (openrange < 56 * FRACUNIT || openbottom - clear_z > 24 * FRACUNIT || ap_is_teleport(l->special)) return clear_ok = 0;
    return true;
}

// A straight walk from the player to x,y crosses nothing it cannot step over.
// Three rays, the centre and either flank of the player's 16-unit radius, so
// a route that grazes a door frame does not count as clear.
static int clear_to(mobj_t *me, fixed_t x, fixed_t y) {
    angle_t a = R_PointToAngle2(me->x, me->y, x, y) + ANG90;
    fixed_t ox = 15 * finecosine[a >> ANGLETOFINESHIFT], oy = 15 * finesine[a >> ANGLETOFINESHIFT];
    clear_ok = 1;
    clear_z = me->z;
    for (int k = -1; k <= 1 && clear_ok; k++)
        P_PathTraverse(me->x + k * ox, me->y + k * oy, x + k * ox, y + k * oy, PT_ADDLINES, clear_cb);
    return clear_ok;
}

// The point to aim for to pass through line l towards (bx, by): across from
// the player but clear of the line's ends, and a little beyond it.
static void portal(mobj_t *me, line_t *l, fixed_t bx, fixed_t by, fixed_t *tx, fixed_t *ty) {
    double dx = l->dx / 65536.0, dy = l->dy / 65536.0, len = sqrt(dx * dx + dy * dy);
    if (len < 1) return;
    double x0 = l->v1->x / 65536.0, y0 = l->v1->y / 65536.0, mx = me->x / 65536.0, my = me->y / 65536.0;
    double t = ((mx - x0) * dx + (my - y0) * dy) / len, m = len > 48 ? 24 : len / 2;
    t = t < m ? m : t > len - m ? len - m : t;
    double px = x0 + dx / len * t, py = y0 + dy / len * t;
    if ((px - mx) * (px - mx) + (py - my) * (py - my) > 96.0 * 96.0) return;
    double nx = dy / len, ny = -dx / len;  // towards the front side
    double sgn = P_PointOnLineSide(bx, by, l) ? -1 : 1;
    *tx = (fixed_t)((px + sgn * nx * 24) * 65536.0);
    *ty = (fixed_t)((py + sgn * ny * 24) * 65536.0);
}

// The nearest point of line l to (x, y).
static void line_point(line_t *l, fixed_t x, fixed_t y, fixed_t *px, fixed_t *py) {
    double dx = l->dx, dy = l->dy, len2 = dx * dx + dy * dy;
    double t = len2 > 0 ? ((double)(x - l->v1->x) * dx + (double)(y - l->v1->y) * dy) / len2 : 0;
    t = t < 0.1 ? 0.1 : t > 0.9 ? 0.9 : t;
    *px = l->v1->x + (fixed_t)(t * dx);
    *py = l->v1->y + (fixed_t)(t * dy);
}

static int angle_diff(angle_t want, angle_t have) { return (int)(want - have) >> 16; }

static void turn_to(ticcmd_t *cmd, mobj_t *me, angle_t want) {
    int diff = angle_diff(want, me->angle);
    if (diff > 2048) diff = 2048;
    if (diff < -2048) diff = -2048;
    cmd->angleturn = (short)diff;
}

// Walk towards angle `go` at `speed` whichever way the player faces.
static void move_along(ticcmd_t *cmd, mobj_t *me, angle_t go, int speed) {
    angle_t rel = go - me->angle;
    int f = rel >> ANGLETOFINESHIFT;
    cmd->forwardmove = (signed char)((speed * finecosine[f]) >> FRACBITS);
    cmd->sidemove = (signed char)(-(speed * finesine[f]) >> FRACBITS);
}

static mobj_t *nearest_enemy(mobj_t *me) {
    mobj_t *best = NULL;
    fixed_t bd = FIGHT_RANGE;
    for (thinker_t *t = thinkercap.next; t != &thinkercap; t = t->next) {
        if (!is_mobj(t)) continue;
        mobj_t *m = (mobj_t *)t;
        if (!(m->flags & MF_COUNTKILL) || m->health <= 0) continue;
        fixed_t d = P_AproxDistance(m->x - me->x, m->y - me->y);
        if (d < bd && P_CheckSight(me, m)) {
            bd = d;
            best = m;
        }
    }
    return best;
}

static void pick_weapon(player_t *p, fixed_t range) {
    static const weapontype_t pref[] = {wp_plasma, wp_chaingun, wp_supershotgun, wp_shotgun, wp_missile, wp_pistol};
    if (p->pendingweapon != wp_nochange) return;
    weapontype_t cur = p->readyweapon;
    int bullets = cur == wp_pistol || cur == wp_chaingun || cur == wp_shotgun || cur == wp_supershotgun;
    int cur_ok = cur != wp_fist && (cur != wp_pistol || shooting) && cur != wp_bfg &&
                 (cur == wp_chainsaw || p->ammo[weaponinfo[cur].ammo] > 0) &&
                 !(cur == wp_missile && range < 320 * FRACUNIT) && (bullets || !shooting);
    if (cur_ok) return;
    for (unsigned i = 0; i < sizeof pref / sizeof *pref; i++) {
        weapontype_t w = pref[i];
        if (shooting && (w == wp_plasma || w == wp_missile)) continue;
        if (w == wp_missile && range < 320 * FRACUNIT) continue;
        if (w == wp_supershotgun && gamemode != commercial) continue;
        if (p->weaponowned[w] && p->ammo[weaponinfo[w].ammo] > 0) {
            if (w != cur) p->pendingweapon = w;
            return;
        }
    }
}

// The errand is done when its thing is gone, its secret found, its line
// pressed or crossed.
static int job_done(player_t *p) {
    mobj_t *me = p->mo;
    switch (job.kind) {
    case K_ITEM: case K_HEALTH: case K_KEY:
        if (!(job.thing->flags & MF_SPECIAL) || job.thing->thinker.function.acp1 != (actionf_p1)P_MobjThinker) return 1;
        if (P_AproxDistance(me->x - job.x, me->y - job.y) < 20 * FRACUNIT) {
            ban_thing(job.thing);  // standing on it and it stays: Doom will not let us take it
            return 1;
        }
        return 0;
    case K_SECRET: return sectors[job.sector].special != 9;
    case K_HUNT:
        return job.thing->health <= 0 || job.thing->thinker.function.acp1 != (actionf_p1)P_MobjThinker ||
               P_AproxDistance(me->x - job.thing->x, me->y - job.thing->y) < 128 * FRACUNIT;
    case K_EXPLORE: return seen[job.cell];
    default:
        // A repeatable walk-over line keeps its special: done once crossed.
        if (job.act == A_CROSS && job.side >= 0 && fx_repeat(lines[job.line].special) &&
            P_PointOnLineSide(me->x, me->y, &lines[job.line]) != job.side &&
            P_AproxDistance(me->x - job.x, me->y - job.y) < 128 * FRACUNIT)
            return 1;
        return lines[job.line].special == 0;
    }
}

static void mark_seen(mobj_t *me) {
    int c = nav_cell(me->x, me->y);
    if (c < 0) return;
    int cx = c % nav_w, cy = c / nav_w, r = 128 / nav_cs;
    for (int y = cy - r; y <= cy + r; y++)
        for (int x = cx - r; x <= cx + r; x++)
            if (x >= 0 && y >= 0 && x < nav_w && y < nav_h) seen[y * nav_w + x] = 1;
}

// A pit or dead end it cannot leave: cut to a random item, which map authors
// only place where a player can stand.
static void hop(mobj_t *me) {
    int n = 0;
    for (thinker_t *t = thinkercap.next; t != &thinkercap; t = t->next)
        if (is_mobj(t) && (((mobj_t *)t)->flags & MF_SPECIAL)) n++;
    if (!n) return;
    // A few draws for an item with room around it: a secret closet or a
    // ledge is no better than the pit.
    mobj_t *pick = NULL;
    for (int tries = 0; tries < 16 && !pick; tries++) {
        int k = (int)(rnd() % (uint32_t)n);
        for (thinker_t *t = thinkercap.next; t != &thinkercap; t = t->next) {
            if (!is_mobj(t) || !(((mobj_t *)t)->flags & MF_SPECIAL) || k--) continue;
            mobj_t *m = (mobj_t *)t;
            if (nav_exits(nav_cell(m->x, m->y), me->player) >= 4 || tries == 15) pick = m;
            break;
        }
    }
    if (!pick) return;
    P_TeleportMove(me, pick->x, pick->y);
    me->z = me->floorz;
    ap_hops++;
}

// At the errand's spot: press its line, walk over it, or step on its thing.
static void finish_job(ticcmd_t *cmd, mobj_t *me, angle_t *face, int *speed, angle_t *go) {
    if (job.act == A_TOUCH || job.line < 0) {
        *go = R_PointToAngle2(me->x, me->y, job.x, job.y);
        *speed = 25;
        return;
    }
    line_t *l = &lines[job.line];
    fixed_t px, py;
    line_point(l, me->x, me->y, &px, &py);
    if (job.act == A_SHOOT) {
        // A gun line: a bullet has only to cross it. Aim at its middle.
        pressing = shooting = 1;
        *face = R_PointToAngle2(me->x, me->y, l->v1->x + l->dx / 2, l->v1->y + l->dy / 2);
        *go = *face;
        *speed = 0;
        if (abs(angle_diff(*face, me->angle)) < 128) cmd->buttons |= BT_ATTACK;
        if (++use_wait > 3 * TIC) {
            use_wait = 0;
            line_ban[job.line] = BANNED;
            job.kind = K_NONE;
            replan_at = 0;
        }
        return;
    }
    if (job.act == A_USE) {
        pressing = 1;
        *face = R_PointToAngle2(me->x, me->y, px, py);
        *go = *face;
        *speed = P_AproxDistance(px - me->x, py - me->y) > 40 * FRACUNIT ? 15 : 0;
        if (abs(angle_diff(*face, me->angle)) < 2048 && leveltime % 6 == 0) cmd->buttons |= BT_USE;
        if (++use_wait > 3 * TIC) {
            use_wait = 0;
            line_ban[job.line] = BANNED;  // pressed, or it will not press: try the next thing
            job.kind = K_NONE;
            replan_at = 0;
        }
        return;
    }
    // Cross: aim through the line to the far side.
    int side = P_PointOnLineSide(me->x, me->y, l);
    fixed_t len = P_AproxDistance(l->dx, l->dy);
    fixed_t nx = FixedDiv(l->dy, len) * 48, ny = -FixedDiv(l->dx, len) * 48;
    *go = R_PointToAngle2(me->x, me->y, side ? px + nx : px - nx, side ? py + ny : py - ny);
    *speed = 30;
    if (++use_wait > 3 * TIC) {
        use_wait = 0;
        line_ban[job.line] = BANNED;
        job.kind = K_NONE;
        replan_at = 0;
    }
}

void AP_Tick(ticcmd_t *cmd) {
    player_t *p = &players[consoleplayer];
    mobj_t *me = p->mo;
    if (!me) return;
    if (gameepisode != level_ep || gamemap != level_map || leveltime < last_time || me != last_mo) {
        last_mo = me;
        level_ep = gameepisode;
        level_map = gamemap;
        rng ^= ((uint32_t)(gameepisode * 977 + gamemap * 131 + gametic) ^ (ap_seed * 2654435761u)) | 1;
        level_start();
    }
    last_time = leveltime;
    mark_seen(me);
    if (leveltime % TIC == 0) nav_decay();

    // Moving at all: 15 s rooted to one spot means a pit, or a loop the
    // planner keeps choosing; jump away.
    if (P_AproxDistance(me->x - anchor_x, me->y - anchor_y) > 96 * FRACUNIT) {
        anchor_x = me->x;
        anchor_y = me->y;
        moved_at = leveltime;
    } else if (leveltime - moved_at > 15 * TIC) {
        hop(me);
        moved_at = leveltime;
        replan_at = 0;
    }

    if (job.kind != K_NONE && job_done(p)) {
#ifdef AP_TRACE
        printf("  t=%5.1f done job %d line %d\n", leveltime / 35.0, job.kind, job.line);
#endif
        if (job.line >= 0 && job.act != A_TOUCH) line_ban[job.line] = BANNED;
        if (job.line >= 0 && ap_is_lift(lines[job.line].special)) ride_until = leveltime + 5 * TIC;
        job.kind = K_NONE;
        replan_at = 0;
    }
    if (job.kind != K_NONE && leveltime - job.since > job.budget) drop_job();
    if (job.kind != K_NONE && job.sub && leveltime >= replan_at) {
        // An errand on the way to the errand: keep it, just re-route.
        nav_flood(nav_cell(me->x, me->y), p);
        plen = nav_path(job.cell, path, PATH_MAX_CELLS);
        pi = 0;
        replan_at = leveltime + REPLAN_TICS;
        if (!plen) drop_job();
    }
    if (job.kind == K_NONE || leveltime >= replan_at) {
        use_wait = job.kind == K_NONE ? 0 : use_wait;
        choose(p);
        if (job.kind == K_NONE && leveltime - moved_at > 3 * TIC && leveltime - hopped_at > 20 * TIC) {
            // Nothing left it can reach: the way on is something it does not
            // model. Give the switches another go, from somewhere else.
            memset(line_ban, 0, numlines);
            hop(me);
            moved_at = hopped_at = leveltime;
        }
    }

    angle_t go = me->angle, face;
    int speed = 0, use_line = -1;
    pressing = shooting = 0;
    fixed_t tx = job.x, ty = job.y;
    int here = nav_cell(me->x, me->y);
    if (plen) {
        // Where the player is along the path; off it entirely means a fall
        // or a push, so plan again.
        int found = -1;
        for (int k = pi; k < plen && k < pi + 16; k++)
            if (path[k] == here) found = k;
        if (found >= 0) pi = found;
        else if (here >= 0 && !clear_to(me, tx, ty) && ++noprog > 20) replan_at = 0;
    }
    int at_end = !plen || pi >= plen - 1;
    if (!at_end) {
        int via;
        if (nav_step(path[pi], path[pi + 1], p, &use_line, &via, &tx, &ty) < 0) {
            replan_at = 0;
        } else {
            // Look ahead along the path for the farthest cell in a straight
            // walk, as far as the next door or lift. Not while stuck: then
            // cell by cell, centre to centre, which never grinds a corner.
            int ahead = noprog > TIC / 2 ? pi + 1 : pi + 4;
            for (int k = ahead < plen - 1 ? ahead : plen - 1; k > pi + 1; k--) {
                fixed_t x, y;
                int u = -1, v, stop = 0;
                for (int j = pi + 1; j < k && !stop; j++) {
                    fixed_t sx, sy;
                    stop = nav_step(path[j], path[j + 1], p, &u, &v, &sx, &sy) < 0 || u != -1;
                }
                if (stop) continue;
                nav_stand(path[k], &x, &y);
                if (clear_to(me, x, y)) {
                    tx = x;
                    ty = y;
                    via = -1;
                    break;
                }
            }
            if (pi + 2 >= plen && clear_to(me, job.x, job.y)) tx = job.x, ty = job.y, via = -1;
            if (noprog > TIC / 2) {
                // Not yet at this cell's centre: go there first.
                fixed_t cx, cy;
                nav_stand(path[pi], &cx, &cy);
                if (P_AproxDistance(cx - me->x, cy - me->y) > 8 * FRACUNIT && !clear_to(me, tx, ty)) tx = cx, ty = cy, via = -1;
            }
            if (via >= 0 && use_line < 0) {
                fixed_t bx, by;
                nav_center(path[pi + 1], &bx, &by);
                portal(me, &lines[via], bx, by, &tx, &ty);
            }
        }
        go = R_PointToAngle2(me->x, me->y, tx, ty);
        speed = 50;
    }
    face = go;

    if (use_line != NAV_WAIT) waited = 0;
    if (use_line == NAV_WAIT && ++waited < 8 * TIC) {
        // A lift on its way, or a door opening: stand still and let it
        // come, for as long as a lift's ride takes; then go round.
        speed = 0;
        noprog = 0;
    } else if (use_line == NAV_WAIT) {
        waited = 0;
        nav_penalize(path[pi + 1]);
        replan_at = 0;
    } else if (use_line >= 0) {
        // A door or lift on the way: walk up to it and press it.
        line_t *l = &lines[use_line];
        fixed_t px, py;
        line_point(l, me->x, me->y, &px, &py);
        fixed_t d = P_AproxDistance(px - me->x, py - me->y);
        if (d < USE_REACH + 40 * FRACUNIT && ap_is_use(l->special)) {
            pressing = 1;
            face = R_PointToAngle2(me->x, me->y, px, py);
            go = face;
            speed = d > USE_REACH - 16 * FRACUNIT ? 20 : 0;
            if (abs(angle_diff(face, me->angle)) < 2048 && leveltime % 8 == 0) cmd->buttons |= BT_USE;
        } else if (!ap_is_use(l->special) || (d > 256 * FRACUNIT && P_PointOnLineSide(me->x, me->y, l) == 0)) {
            // The lift's switch is somewhere else, or it is a line to walk
            // over: go and press or cross that first.
            cand_t c = {K_NONE, A_TOUCH, -1, -1, -1, NAV_FAR, NULL, 0, 0};
            offer_line(&c, K_SWITCH, use_line, ap_is_use(l->special) ? A_USE : A_CROSS);
            c.line = use_line;
            if (c.d < NAV_FAR && line_ban[use_line] < BANNED) set_job(&c, 1);
        }
    } else if (at_end && job.kind != K_NONE) {
        finish_job(cmd, me, &face, &speed, &go);
    }

    // Progress: less of the path left, plus the way to the waypoint. Without
    // it for a second, jiggle and press use; for three, make the next cell
    // expensive and plan around it.
    fixed_t wd = P_AproxDistance(tx - me->x, ty - me->y) + (plen - pi) * nav_cs * FRACUNIT;
    if (speed == 0 || wd < best_d - 8 * FRACUNIT) {
        best_d = speed ? wd : best_d;
        noprog = 0;
    } else if (++noprog > TIC) {
        if (noprog == TIC + 1) jiggle = (rnd() & 1) ? 1 : -1;
        cmd->buttons |= BT_USE;
        if (noprog > 3 * TIC) {
            if (pi + 1 < plen) nav_penalize(path[pi + 1]);
            // Three strikes against a line or a secret, one against a thing.
            if (job.line >= 0 && ++line_ban[job.line] >= BANNED) drop_job();
            else if (job.sector >= 0 && ++sec_ban[job.sector] >= BANNED) drop_job();
            else if (job.line < 0 && job.sector < 0) drop_job();
            replan_at = 0;
            noprog = 0;
            best_d = NAV_FAR;
        }
    }
    mobj_t *enemy = nearest_enemy(me);
    if (enemy) {
        // Pressing a switch or door wins over turning to fight; it still
        // fires at whatever is in front of it.
        fixed_t range = P_AproxDistance(enemy->x - me->x, enemy->y - me->y);
        angle_t at = R_PointToAngle2(me->x, me->y, enemy->x, enemy->y);
        pick_weapon(p, range);
        if (abs(angle_diff(at, me->angle)) < 1024) cmd->buttons |= BT_ATTACK;
        if (!pressing) face = at;
    } else {
        pick_weapon(p, FIGHT_RANGE);
    }
    // Just set a lift off from on top of it: ride it down rather than step off.
    if (leveltime < ride_until && me->subsector->sector->specialdata) speed = 0, noprog = 0;
    turn_to(cmd, me, face);
    move_along(cmd, me, go, speed);
    if (noprog > TIC && jiggle) cmd->sidemove = (signed char)(jiggle * 40);
}

// What a switch, walk-over or gun line will do to the level: the sectors it
// tags and the heights they settle at, laid over the real ones as a
// hypothesis the nav grid can be flooded through.
//
// Copyright (C) 2026 Adam Hitchcock. GPL-2.0-or-later, like the engine it
// links against; see doomgeneric/LICENSE.
//
// The idea, not the code, is AutoDoom's (ioan-chera/AutoDoom,
// b_lineeffect.cpp): push a line's effect onto a stack of sector heights,
// search, pop. Targets follow p_floor.c, p_plats.c, p_doors.c and
// p_ceilng.c, computed from the hypothesis's heights so pushes chain.

#include <stdlib.h>

#include "autopilot.h"
#include "doomstat.h"
#include "p_local.h"
#include "p_spec.h"
#include "r_state.h"

enum {
    X_NONE,
    X_OPEN,       // ceiling up to the lowest around it, less 4
    X_TIMED,      // the same, then it shuts again
    X_SHUT,       // ceiling down to the floor
    X_CEIL_FLOOR, // ceiling to floor + 8 (crusher that stays)
    X_CEIL_HI,    // ceiling to the highest around it
    X_HI_FLOOR,   // floor to the highest floor around it
    X_LO_FLOOR,   // floor to the lowest floor around it
    X_TURBO,      // highest around, plus 8 if not already there
    X_RAISE,      // floor to the lowest ceiling around it
    X_RAISE_CRUSH,
    X_NEXT,       // floor to the next higher floor around it
    X_UP24,
    X_UP32,
    X_UP512,
    X_TEXTURE,    // up by its shortest lower texture
    X_STAIRS8,
    X_STAIRS16,
    X_LIFT,       // down to the lowest around it, then back
    X_EXIT,
};

static int ngen = 1;
static int *ogen;
static fixed_t *ofloor, *oceil, *ohigh;
static unsigned char *movable;
static int cap;

// The trigger and action of each special Doom has: walk (W), switch (S) or
// gun (G), once (1) or repeatable (R).
static void classify(int s, int *trig, int *rep, int *act) {
    int t = FX_WALK, r = 0, a = X_NONE;
    switch (s) {
    case 2: a = X_OPEN; break;
    case 3: a = X_SHUT; break;
    case 4: a = X_TIMED; break;
    case 5: a = X_RAISE; break;
    case 8: a = X_STAIRS8; break;
    case 10: a = X_LIFT; break;
    case 19: a = X_HI_FLOOR; break;
    case 22: a = X_NEXT; break;
    case 30: a = X_TEXTURE; break;
    case 36: a = X_TURBO; break;
    case 37: case 38: a = X_LO_FLOOR; break;
    case 40: a = X_CEIL_HI; break;
    case 44: a = X_CEIL_FLOOR; break;
    case 52: a = X_EXIT; break;
    case 53: a = X_LIFT; break;
    case 56: a = X_RAISE_CRUSH; break;
    case 58: case 59: a = X_UP24; break;
    case 100: a = X_STAIRS16; break;
    case 108: a = X_TIMED; break;
    case 109: a = X_OPEN; break;
    case 110: a = X_SHUT; break;
    case 119: case 130: a = X_NEXT; break;
    case 121: a = X_LIFT; break;
    case 124: a = X_EXIT; break;
    case 72: r = 1, a = X_CEIL_FLOOR; break;
    case 75: case 107: r = 1, a = X_SHUT; break;
    case 82: case 84: r = 1, a = X_LO_FLOOR; break;
    case 83: r = 1, a = X_HI_FLOOR; break;
    case 86: case 106: r = 1, a = X_OPEN; break;
    case 87: case 88: case 120: r = 1, a = X_LIFT; break;
    case 90: case 105: r = 1, a = X_TIMED; break;
    case 91: r = 1, a = X_RAISE; break;
    case 92: case 93: r = 1, a = X_UP24; break;
    case 94: r = 1, a = X_RAISE_CRUSH; break;
    case 95: case 128: case 129: r = 1, a = X_NEXT; break;
    case 96: r = 1, a = X_TEXTURE; break;
    case 98: r = 1, a = X_TURBO; break;
    default:
        t = FX_USE;
        switch (s) {
        case 7: a = X_STAIRS8; break;
        case 11: case 51: a = X_EXIT; break;
        case 14: a = X_UP32; break;
        case 15: a = X_UP24; break;
        case 18: case 20: case 131: a = X_NEXT; break;
        case 21: case 122: a = X_LIFT; break;
        case 23: a = X_LO_FLOOR; break;
        case 29: case 111: a = X_TIMED; break;
        case 41: a = X_SHUT; break;
        case 50: case 113: a = X_SHUT; break;
        case 55: a = X_RAISE_CRUSH; break;
        case 71: a = X_TURBO; break;
        case 101: a = X_RAISE; break;
        case 102: a = X_HI_FLOOR; break;
        case 103: case 112: case 133: case 135: case 137: a = X_OPEN; break;
        case 127: a = X_STAIRS16; break;
        case 140: a = X_UP512; break;
        case 42: case 43: case 116: r = 1, a = X_SHUT; break;
        case 45: r = 1, a = X_HI_FLOOR; break;
        case 60: r = 1, a = X_LO_FLOOR; break;
        case 61: case 99: case 115: case 134: case 136: r = 1, a = X_OPEN; break;
        case 62: case 123: r = 1, a = X_LIFT; break;
        case 63: case 114: r = 1, a = X_TIMED; break;
        case 64: r = 1, a = X_RAISE; break;
        case 65: r = 1, a = X_RAISE_CRUSH; break;
        case 66: r = 1, a = X_UP24; break;
        case 67: r = 1, a = X_UP32; break;
        case 68: case 69: case 132: r = 1, a = X_NEXT; break;
        case 70: r = 1, a = X_TURBO; break;
        default:
            t = FX_SHOOT;
            switch (s) {
            case 24: a = X_RAISE; break;
            case 46: r = 1, a = X_OPEN; break;
            case 47: a = X_NEXT; break;
            default: t = 0; break;
            }
        }
    }
    *trig = t;
    *rep = r;
    *act = a;
}

int fx_trigger(int special) {
    int t, r, a;
    classify(special, &t, &r, &a);
    return a == X_NONE ? 0 : t;
}

int fx_repeat(int special) {
    int t, r, a;
    classify(special, &t, &r, &a);
    return r;
}

int fx_shuts(int special) {
    int t, r, a;
    classify(special, &t, &r, &a);
    return a == X_SHUT || a == X_CEIL_FLOOR;
}

static int on(int s) { return ogen[s] == ngen; }
fixed_t fx_floor(int s) { return on(s) ? ofloor[s] : sectors[s].floorheight; }
fixed_t fx_floor_hi(int s) { return on(s) ? ohigh[s] : sectors[s].floorheight; }
int fx_ranged(int s) { return on(s) && ohigh[s] != ofloor[s]; }
fixed_t fx_ceil(int s) { return on(s) ? oceil[s] : sectors[s].ceilingheight; }
int fx_moves(int s) { return movable[s]; }
int fx_active(int s) { return on(s); }

static void set(int s, fixed_t f, fixed_t c) {
    ofloor[s] = ohigh[s] = f;
    oceil[s] = c;
    ogen[s] = ngen;
}

static int other(line_t *l, int s) {
    if (!(l->flags & ML_TWOSIDED) || !l->backsector) return -1;
    int f = (int)(l->frontsector - sectors), b = (int)(l->backsector - sectors);
    return f == s ? b : f;
}

// mode 0: highest floor around, 1: lowest floor around (start at its own),
// 2: lowest ceiling around, 3: highest ceiling around, 4: next floor above h.
static fixed_t around(int s, int mode, fixed_t h) {
    sector_t *S = &sectors[s];
    fixed_t best = mode == 0 ? -500 * FRACUNIT : mode == 1 ? fx_floor(s) : mode == 2 ? INT32_MAX : mode == 3 ? 0 : h;
    int found = 0;
    for (int i = 0; i < S->linecount; i++) {
        int o = other(S->lines[i], s);
        if (o < 0) continue;
        fixed_t f = fx_floor(o), c = fx_ceil(o);
        switch (mode) {
        case 0: if (f > best) best = f; break;
        case 1: if (f < best) best = f; break;
        case 2: if (c < best) best = c; break;
        case 3: if (c > best) best = c; break;
        default:
            if (f > h && (!found || f < best)) best = f, found = 1;
            break;
        }
    }
    return best;
}

static fixed_t shortest_lower(int s) {
    sector_t *S = &sectors[s];
    int best = INT32_MAX;
    for (int i = 0; i < S->linecount; i++) {
        line_t *l = S->lines[i];
        if (!(l->flags & ML_TWOSIDED)) continue;
        for (int k = 0; k < 2; k++) {
            int t = sides[l->sidenum[k]].bottomtexture;
            if (t >= 0 && textureheight[t] < best) best = textureheight[t];
        }
    }
    return best == INT32_MAX ? 64 * FRACUNIT : best;
}

static int stairs(int s, fixed_t size) {
    short pic = sectors[s].floorpic;
    fixed_t h = fx_floor(s) + size;
    int n = 0;
    for (int guard = 0; guard < numsectors; guard++) {
        set(s, h, fx_ceil(s));
        n++;
        int next = -1;
        sector_t *S = &sectors[s];
        for (int i = 0; i < S->linecount && next < 0; i++) {
            line_t *l = S->lines[i];
            if (!(l->flags & ML_TWOSIDED) || l->frontsector != S || l->backsector->floorpic != pic) continue;
            next = (int)(l->backsector - sectors);
        }
        if (next < 0 || on(next)) break;
        s = next;
        h += size;
    }
    return n;
}

// One sector's share of the action; 1 if its heights change.
static int act_on(int s, int a) {
    fixed_t f = fx_floor(s), c = fx_ceil(s), nf = f, nc = c;
    switch (a) {
    case X_OPEN: case X_TIMED: nc = around(s, 2, 0) - 4 * FRACUNIT; break;
    case X_SHUT: nc = f; break;
    case X_CEIL_FLOOR: nc = f + 8 * FRACUNIT; break;
    case X_CEIL_HI: nc = around(s, 3, 0); break;
    case X_HI_FLOOR: nf = around(s, 0, 0); break;
    case X_LO_FLOOR: nf = around(s, 1, 0); break;
    case X_LIFT:
        // Down and back up: both heights, for as long as the ride takes.
        nf = around(s, 1, 0);
        if (nf == f) return 0;
        set(s, nf, c);
        ohigh[s] = f;
        return 1;
    case X_TURBO:
        nf = around(s, 0, 0);
        if (nf != f) nf += 8 * FRACUNIT;
        break;
    case X_RAISE: case X_RAISE_CRUSH:
        nf = around(s, 2, 0);
        if (nf > c) nf = c;
        if (a == X_RAISE_CRUSH) nf -= 8 * FRACUNIT;
        break;
    case X_NEXT: nf = around(s, 4, f); break;
    case X_UP24: nf = f + 24 * FRACUNIT; break;
    case X_UP32: nf = f + 32 * FRACUNIT; break;
    case X_UP512: nf = f + 512 * FRACUNIT; break;
    case X_TEXTURE: nf = f + shortest_lower(s); break;
    case X_STAIRS8: return stairs(s, 8 * FRACUNIT);
    case X_STAIRS16: return stairs(s, 16 * FRACUNIT);
    default: return 0;
    }
    if (nf == f && nc == c) return 0;
    set(s, nf, nc);
    return 1;
}

int fx_push(int li) {
    line_t *l = &lines[li];
    int t, r, a;
    classify(l->special, &t, &r, &a);
    if (a == X_NONE || a == X_EXIT || !l->tag) return 0;
    int n = 0;
    for (int s = -1; (s = P_FindSectorFromLineTag(l, s)) >= 0;)
        if (!sectors[s].specialdata || on(s)) n += act_on(s, a);
    return n;
}

void fx_reset(void) { ngen++; }

void fx_build(void) {
    if (numsectors > cap) {
        cap = numsectors;
        ogen = realloc(ogen, cap * sizeof *ogen);
        ofloor = realloc(ofloor, cap * sizeof *ofloor);
        oceil = realloc(oceil, cap * sizeof *oceil);
        ohigh = realloc(ohigh, cap * sizeof *ohigh);
        movable = realloc(movable, cap);
        if (!ogen || !ofloor || !oceil || !ohigh || !movable) abort();
    }
    ngen = 1;
    for (int i = 0; i < numsectors; i++) ogen[i] = 0, movable[i] = 0;
    for (int i = 0; i < numlines; i++) {
        line_t *l = &lines[i];
        int t, r, a;
        classify(l->special, &t, &r, &a);
        if (a == X_NONE || a == X_EXIT || !l->tag) continue;
        for (int s = -1; (s = P_FindSectorFromLineTag(l, s)) >= 0;) {
            movable[s] = 1;
            if (a == X_STAIRS8 || a == X_STAIRS16) {
                // The whole flight moves, not only the first step.
                ngen++;
                stairs(s, 0);
                for (int k = 0; k < numsectors; k++)
                    if (on(k)) movable[k] = 1;
            }
        }
    }
    ngen++;
}

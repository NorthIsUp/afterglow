// The autopilot's map of the level: a grid of 32-unit cells over the
// linedefs, searched with Dijkstra.
//
// Copyright (C) 2026 Adam Hitchcock. GPL-2.0-or-later, like the engine it
// links against; see doomgeneric/LICENSE.
//
// A cell is its centre's sector. An edge to one of the eight neighbours lists
// the two-sided lines its segment crosses, found once per edge with
// P_PathTraverse and cached; one-sided and blocking lines close it for good.
// Whether the player can cross is decided at search time from the sectors'
// current heights, because doors open, lifts move and stairs rise. A door the
// player can open (with the key, if locked) counts as open, and so does a lift
// it can call, so a route through them is found before they move. A cell with
// no spot clear of walls and ledges for the player's 16-unit radius is closed.

#include <stdlib.h>
#include <string.h>

#include "autopilot.h"
#include "doomstat.h"
#include "info.h"
#include "p_local.h"
#include "p_spec.h"
#include "r_main.h"
#include "r_state.h"

#define STEP (24 * FRACUNIT)
#define HEADROOM (56 * FRACUNIT)
#define MAXDIM 384
#define HITS 4

enum { E_DONE = 1, E_WALL = 2, E_MORE = 4 };
enum { C_TIGHT = 1, C_TIGHTDONE = 2, C_THING = 4, C_STANDDONE = 8 };

typedef struct {
    short line[HITS];
    unsigned char n, flags;
} edge_t;

int nav_w, nav_h, nav_cs;
static fixed_t orgx, orgy;
static int ncells, cap;
static short *csec;
static unsigned char *cflags, *penalty;
static signed char (*standoff)[2];  // per cell: where to stand, off its centre (127: nowhere)
static edge_t *edges;  // 4 per cell: E, NE, N, NW
static int *dist, *par, *gen, *heap, *hpos;
static signed char *pdir;
static int flood_gen, hlen, walk_via;

// Per sector, computed at build: the line that opens it as a door (or -1)
// and its open height; its range as a lift and the line that calls it.
static int nsec_cap;
static short *door_line, *lift_line;
static fixed_t *door_top, *lift_lo, *lift_hi;
static int *tele_dest;
static int nline_cap;

static const int DX[8] = {1, 1, 0, -1, -1, -1, 0, 1};
static const int DY[8] = {0, 1, 1, 1, 0, -1, -1, -1};

int ap_door_key(int s) {
    switch (s) {
    case 1: case 31: case 117: case 118: return 0;
    case 26: case 32: return 1;
    case 27: case 34: return 2;
    case 28: case 33: return 3;
    default: return -1;
    }
}

int ap_lock_key(int s) {
    switch (s) {
    case 99: case 133: return 1;
    case 136: case 137: return 2;
    case 134: case 135: return 3;
    default: return 0;
    }
}

int ap_has_key(player_t *p, int key) {
    switch (key) {
    case 1: return p->cards[it_bluecard] || p->cards[it_blueskull];
    case 2: return p->cards[it_yellowcard] || p->cards[it_yellowskull];
    case 3: return p->cards[it_redcard] || p->cards[it_redskull];
    default: return 1;
    }
}

int ap_is_lift(int s) {
    switch (s) {
    case 10: case 21: case 53: case 62: case 87: case 88:
    case 120: case 121: case 122: case 123: return 1;
    default: return 0;
    }
}

int ap_is_exit(int s) {
    return s == 11 || s == 52 ? 1 : s == 51 || s == 124 ? 2 : 0;
}

int ap_is_use(int s) {
    if (ap_door_key(s) >= 0) return 1;
    switch (s) {
    case 7: case 9: case 11: case 14: case 15: case 18: case 20: case 21:
    case 23: case 29: case 41: case 42: case 43: case 45: case 49: case 50:
    case 51: case 55: case 60: case 61: case 62: case 63: case 64: case 65:
    case 66: case 67: case 68: case 69: case 70: case 71: case 99: case 101:
    case 102: case 103: case 111: case 112: case 113: case 114: case 115:
    case 116: case 122: case 123: case 127: case 131: case 132: case 133:
    case 134: case 135: case 136: case 137: case 138: case 139: case 140:
        return 1;
    default: return 0;
    }
}

int ap_is_teleport(int s) { return s == 39 || s == 97; }

static void *grow(void *p, size_t n) {
    void *q = realloc(p, n);
    if (!q) abort();
    return q;
}

static int sec_index(sector_t *s) { return s ? (int)(s - sectors) : -1; }

// How good a line is at calling lift sector s from below: a switch on its
// edge, a switch elsewhere, a walk-over line away from it, one on its edge.
static int lift_rank(line_t *l, int s) {
    int edge = l->frontsector == &sectors[s] || l->backsector == &sectors[s];
    if (ap_is_use(l->special)) return edge ? 4 : 3;
    return edge ? 1 : 2;
}

static void build_sectors(void) {
    if (numsectors > nsec_cap) {
        nsec_cap = numsectors;
        door_line = grow(door_line, nsec_cap * sizeof *door_line);
        lift_line = grow(lift_line, nsec_cap * sizeof *lift_line);
        door_top = grow(door_top, nsec_cap * sizeof *door_top);
        lift_lo = grow(lift_lo, nsec_cap * sizeof *lift_lo);
        lift_hi = grow(lift_hi, nsec_cap * sizeof *lift_hi);
    }
    if (numlines > nline_cap) {
        nline_cap = numlines;
        tele_dest = grow(tele_dest, nline_cap * sizeof *tele_dest);
    }
    for (int i = 0; i < numsectors; i++) {
        door_line[i] = lift_line[i] = -1;
        door_top[i] = P_FindLowestCeilingSurrounding(&sectors[i]) - 4 * FRACUNIT;
        lift_lo[i] = P_FindLowestFloorSurrounding(&sectors[i]);
        if (lift_lo[i] > sectors[i].floorheight) lift_lo[i] = sectors[i].floorheight;
        lift_hi[i] = sectors[i].floorheight;
    }
    for (int i = 0; i < numlines; i++) {
        line_t *l = &lines[i];
        tele_dest[i] = -2;
        if (ap_door_key(l->special) >= 0 && l->backsector) door_line[sec_index(l->backsector)] = i;
        if (ap_is_lift(l->special) && l->tag)
            for (int s = -1; (s = P_FindSectorFromLineTag(l, s)) >= 0;)
                if (lift_rank(l, s) > (lift_line[s] < 0 ? 0 : lift_rank(&lines[lift_line[s]], s))) lift_line[s] = i;
    }
    // A lift whose only call is a walk-over line on its own edge can be
    // ridden down but never called from below: Doom will not let the
    // player's radius over that edge while the lift is up.
    for (int i = 0; i < numsectors; i++)
        if (lift_line[i] >= 0 && lift_rank(&lines[lift_line[i]], i) == 1) lift_line[i] = -1;
}

int nav_cell(fixed_t x, fixed_t y) {
    int cx = (int)(((int64_t)x - orgx) / (nav_cs * FRACUNIT));
    int cy = (int)(((int64_t)y - orgy) / (nav_cs * FRACUNIT));
    if (x < orgx || y < orgy || cx >= nav_w || cy >= nav_h) return NAV_NONE;
    return cy * nav_w + cx;
}

void nav_center(int c, fixed_t *x, fixed_t *y) {
    *x = orgx + (c % nav_w) * nav_cs * FRACUNIT + nav_cs * FRACUNIT / 2;
    *y = orgy + (c / nav_w) * nav_cs * FRACUNIT + nav_cs * FRACUNIT / 2;
}

int nav_sector(int c) { return c < 0 ? -1 : csec[c]; }

void nav_build(void) {
    fixed_t x0 = INT32_MAX, y0 = INT32_MAX, x1 = INT32_MIN, y1 = INT32_MIN;
    for (int i = 0; i < numvertexes; i++) {
        if (vertexes[i].x < x0) x0 = vertexes[i].x;
        if (vertexes[i].y < y0) y0 = vertexes[i].y;
        if (vertexes[i].x > x1) x1 = vertexes[i].x;
        if (vertexes[i].y > y1) y1 = vertexes[i].y;
    }
    int span = ((x1 - x0) > (y1 - y0) ? (x1 - x0) : (y1 - y0)) >> FRACBITS;
    nav_cs = 32;
    while (span / nav_cs >= MAXDIM) nav_cs += 16;
    // Off the 8-unit grid map authors draw on, so a cell's centre is never
    // on a line, where its sector and side would be a coin toss.
    orgx = x0 - 5 * FRACUNIT;
    orgy = y0 - 3 * FRACUNIT;
    nav_w = ((x1 - orgx) >> FRACBITS) / nav_cs + 1;
    nav_h = ((y1 - orgy) >> FRACBITS) / nav_cs + 1;
    ncells = nav_w * nav_h;
    if (ncells > cap) {
        cap = ncells;
        csec = grow(csec, cap * sizeof *csec);
        cflags = grow(cflags, cap);
        standoff = grow(standoff, cap * sizeof *standoff);
        penalty = grow(penalty, cap);
        edges = grow(edges, cap * 4 * sizeof *edges);
        dist = grow(dist, cap * sizeof *dist);
        par = grow(par, cap * sizeof *par);
        gen = grow(gen, cap * sizeof *gen);
        heap = grow(heap, cap * sizeof *heap);
        hpos = grow(hpos, cap * sizeof *hpos);
        pdir = grow(pdir, cap);
    }
    memset(cflags, 0, ncells);
    memset(penalty, 0, ncells);
    memset(gen, 0, ncells * sizeof *gen);
    for (int i = 0; i < ncells * 4; i++) edges[i].flags = 0;
    flood_gen = 0;
    for (int c = 0; c < ncells; c++) {
        fixed_t x, y;
        nav_center(c, &x, &y);
        csec[c] = (short)sec_index(R_PointInSubsector(x, y)->sector);
    }
    // Solid scenery (pillars, lamps, barrels) blocks the cells it covers.
    for (thinker_t *t = thinkercap.next; t != &thinkercap; t = t->next) {
        if (t->function.acp1 != (actionf_p1)P_MobjThinker) continue;
        mobj_t *m = (mobj_t *)t;
        if (!(m->flags & MF_SOLID) || (m->flags & MF_COUNTKILL) || m->player) continue;
        int r = (m->radius >> FRACBITS) + 12;
        for (int dy = -r; dy <= r; dy += nav_cs / 2)
            for (int dx = -r; dx <= r; dx += nav_cs / 2) {
                int c = nav_cell(m->x + dx * FRACUNIT, m->y + dy * FRACUNIT);
                fixed_t cx, cy;
                if (c < 0) continue;
                nav_center(c, &cx, &cy);
                if (abs((cx - m->x) >> FRACBITS) < r && abs((cy - m->y) >> FRACBITS) < r) cflags[c] |= C_THING;
            }
    }
    build_sectors();
}

// ---- edges ----

static short hits[HITS];
static int nhit, hit_wall, hit_more;

static boolean collect(intercept_t *in) {
    line_t *l = in->d.line;
    if (!l->backsector || (l->flags & ML_BLOCKING)) {
        hit_wall = 1;
        return false;
    }
    if (l->frontsector == l->backsector && !l->special) return true;
    if (nhit < HITS) hits[nhit++] = (short)(l - lines);
    else hit_more = 1;
    return true;
}

static boolean side_wall(intercept_t *in) {
    line_t *l = in->d.line;
    if (!l->backsector || (l->flags & ML_BLOCKING)) {
        hit_wall = 1;
        return false;
    }
    return true;
}

// The segment between two centres, and either side of it 12 units out: a gap
// narrower than about 56 units does not let the player's 32 through.
static void trace_edge(int a, int b) {
    fixed_t ax, ay, bx, by;
    nav_center(a, &ax, &ay);
    nav_center(b, &bx, &by);
    nhit = hit_wall = hit_more = 0;
    P_PathTraverse(ax, ay, bx, by, PT_ADDLINES, collect);
    int sx = (by > ay) - (by < ay), sy = (ax > bx) - (ax < bx);  // perpendicular
    fixed_t ox = sx * 12 * FRACUNIT, oy = sy * 12 * FRACUNIT;
    if (sx && sy) ox = ox * 7 / 10, oy = oy * 7 / 10;
    for (int k = -1; k <= 1 && !hit_wall; k += 2)
        P_PathTraverse(ax + k * ox, ay + k * oy, bx + k * ox, by + k * oy, PT_ADDLINES, side_wall);
}

// The cached edge for direction d out of cell a, d in 0..7; *rev when it is
// stored on the neighbour and its lines run backwards.
static edge_t *edge(int a, int d, int *rev) {
    int b = a + DY[d] * nav_w + DX[d];
    int owner = d < 4 ? a : b, fd = d & 3;
    edge_t *e = &edges[owner * 4 + fd];
    *rev = d >= 4;
    if (!(e->flags & E_DONE)) {
        trace_edge(owner, owner + DY[fd] * nav_w + DX[fd]);
        e->flags = E_DONE | (hit_wall ? E_WALL : 0) | (hit_more ? E_MORE : 0);
        e->n = (unsigned char)nhit;
        memcpy(e->line, hits, sizeof hits);
    }
    return e;
}

static double near_x, near_y, near_r;
static int near_hit;

static double seg_dist2(line_t *l) {
    double x0 = l->v1->x / 65536.0, y0 = l->v1->y / 65536.0;
    double dx = l->dx / 65536.0, dy = l->dy / 65536.0, len2 = dx * dx + dy * dy;
    double t = len2 > 0 ? ((near_x - x0) * dx + (near_y - y0) * dy) / len2 : 0;
    t = t < 0 ? 0 : t > 1 ? 1 : t;
    double ex = x0 + t * dx - near_x, ey = y0 + t * dy - near_y;
    return ex * ex + ey * ey;
}

static int stand_sec, stand_bad;

// Blocks the player's 16-unit radius at the probe point: a wall, or a line
// with a step up or a low ceiling beyond it (a door or lift will move).
static boolean stand_cb(line_t *l) {
    if (seg_dist2(l) >= near_r * near_r) return true;
    if (!l->backsector || (l->flags & ML_BLOCKING)) return stand_bad = 1, false;
    if (l->frontsector == l->backsector) return true;
    int side = P_PointOnLineSide((fixed_t)(near_x * 65536), (fixed_t)(near_y * 65536), l);
    int there = sec_index(side ? l->frontsector : l->backsector);
    if (there == stand_sec || lift_line[there] >= 0 || door_line[there] >= 0) return true;
    sector_t *T = &sectors[there], *H = &sectors[stand_sec];
    if (T->floorheight - H->floorheight > STEP || T->ceilingheight - H->floorheight < HEADROOM) return stand_bad = 1, false;
    return true;
}

static int stand_ok(fixed_t x, fixed_t y, int sec) {
    near_x = x / 65536.0;
    near_y = y / 65536.0;
    near_r = 16;
    stand_sec = sec;
    stand_bad = 0;
    int x0 = (x - 16 * FRACUNIT - bmaporgx) >> MAPBLOCKSHIFT, x1 = (x + 16 * FRACUNIT - bmaporgx) >> MAPBLOCKSHIFT;
    int y0 = (y - 16 * FRACUNIT - bmaporgy) >> MAPBLOCKSHIFT, y1 = (y + 16 * FRACUNIT - bmaporgy) >> MAPBLOCKSHIFT;
    validcount++;
    for (int by = y0; by <= y1 && !stand_bad; by++)
        for (int bx = x0; bx <= x1 && !stand_bad; bx++) P_BlockLinesIterator(bx, by, stand_cb);
    return !stand_bad && sec_index(R_PointInSubsector(x, y)->sector) == sec;
}

// Where in cell c the player can stand clear of walls and ledges: its
// centre, or a point up to 12 units off it. A cell with none is closed: a
// path through it would have the player grinding a wall or a step it cannot
// climb, its nose to the corner.
static int stand(int c) {
    if (!(cflags[c] & C_STANDDONE)) {
        static const signed char off[13][2] = {{0, 0}, {8, 0}, {-8, 0}, {0, 8}, {0, -8}, {6, 6}, {-6, 6}, {6, -6}, {-6, -6},
                                               {12, 0}, {-12, 0}, {0, 12}, {0, -12}};
        fixed_t x, y;
        nav_center(c, &x, &y);
        standoff[c][0] = standoff[c][1] = 127;
        for (int i = 0; i < 13; i++)
            if (stand_ok(x + off[i][0] * FRACUNIT, y + off[i][1] * FRACUNIT, csec[c])) {
                standoff[c][0] = off[i][0];
                standoff[c][1] = off[i][1];
                break;
            }
        cflags[c] |= C_STANDDONE;
    }
    return standoff[c][0] != 127;
}

void nav_stand(int c, fixed_t *x, fixed_t *y) {
    nav_center(c, x, y);
    if (c >= 0 && stand(c)) {
        *x += standoff[c][0] * FRACUNIT;
        *y += standoff[c][1] * FRACUNIT;
    }
}

static boolean near_wall(line_t *l) {
    if (l->backsector && !(l->flags & ML_BLOCKING)) return true;
    if (seg_dist2(l) >= near_r * near_r) return true;
    near_hit = 1;
    return false;
}

// Within 20 units of a wall: walkable, but routes keep off it so the player
// is not snagged on door frames and does not scrape along with its nose to
// the wall.
static int tight(int c) {
    if (!(cflags[c] & C_TIGHTDONE)) {
        fixed_t x, y;
        nav_center(c, &x, &y);
        near_x = x / 65536.0;
        near_y = y / 65536.0;
        near_r = 20;
        near_hit = 0;
        int x0 = (x - 20 * FRACUNIT - bmaporgx) >> MAPBLOCKSHIFT, x1 = (x + 20 * FRACUNIT - bmaporgx) >> MAPBLOCKSHIFT;
        int y0 = (y - 20 * FRACUNIT - bmaporgy) >> MAPBLOCKSHIFT, y1 = (y + 20 * FRACUNIT - bmaporgy) >> MAPBLOCKSHIFT;
        validcount++;
        for (int by = y0; by <= y1 && !near_hit; by++)
            for (int bx = x0; bx <= x1 && !near_hit; bx++) P_BlockLinesIterator(bx, by, near_wall);
        cflags[c] |= C_TIGHTDONE | (near_hit ? C_TIGHT : 0);
    }
    return cflags[c] & C_TIGHT;
}

static fixed_t ceil_open(int s) {
    fixed_t c = sectors[s].ceilingheight;
    return door_line[s] >= 0 && door_top[s] > c ? door_top[s] : c;
}

static fixed_t floor_lo(int s) {
    return lift_line[s] >= 0 && lift_lo[s] < sectors[s].floorheight ? lift_lo[s] : sectors[s].floorheight;
}

static fixed_t floor_hi(int s) {
    return lift_line[s] >= 0 && lift_hi[s] > sectors[s].floorheight ? lift_hi[s] : sectors[s].floorheight;
}

// Over line li from its side `side`. Returns the sector beyond, or -1 if the
// player cannot get there; *use is the line to press or cross first.
static int cross(int side, int li, player_t *p, int *use, int *cost) {
    line_t *l = &lines[li];
    int s = sec_index(side ? l->backsector : l->frontsector);
    int n = sec_index(side ? l->frontsector : l->backsector);
    if (n == s) return n;
    sector_t *S = &sectors[s], *N = &sectors[n];
    fixed_t top = ceil_open(s) < ceil_open(n) ? ceil_open(s) : ceil_open(n);
    fixed_t bot = floor_lo(s) > floor_lo(n) ? floor_lo(s) : floor_lo(n);
    if (floor_lo(n) - floor_hi(s) > STEP || top - bot < HEADROOM) return -1;
    fixed_t ctop = S->ceilingheight < N->ceilingheight ? S->ceilingheight : N->ceilingheight;
    fixed_t cbot = S->floorheight > N->floorheight ? S->floorheight : N->floorheight;
    int now = N->floorheight - S->floorheight <= STEP && ctop - cbot >= HEADROOM;
    if (now) return n;
    *cost += 64;
    if (N->specialdata || S->specialdata) {
        *use = NAV_WAIT;  // moving already: wait for it
        return n;
    }
    int dkey = ap_door_key(l->special);
    if (door_line[n] >= 0 && N->ceilingheight - cbot < HEADROOM) {
        if (dkey < 0 || l->backsector != N || !ap_has_key(p, dkey)) return -1;
        *use = li;
        return n;
    }
    if (lift_line[n] >= 0 && N->floorheight - S->floorheight > STEP) {
        // The line being crossed may call the lift itself.
        int li2 = ap_is_lift(l->special) && ap_is_use(l->special) && l->tag == N->tag ? li : lift_line[n];
        line_t *a = &lines[li2];
        *use = li2;
        *cost += a->frontsector == N || a->backsector == N ? 0 : 256;
        return n;
    }
    // A door the player is in, or a lift it stands on: a lift that is not
    // moving has to be called up again.
    if (lift_line[s] >= 0) {
        // Only a switch that faces onto the lift can be pressed from it.
        line_t *a = &lines[lift_line[s]];
        if (ap_is_use(a->special) && (a->frontsector == S) != (a->backsector == S) && a->frontsector != S) return -1;
        *use = lift_line[s];
        return n;
    }
    if (door_line[s] < 0) return -1;
    *use = NAV_WAIT;
    return n;
}

static int tele_cell(int li) {
    if (tele_dest[li] == -2) {
        tele_dest[li] = -1;
        line_t *l = &lines[li];
        for (thinker_t *t = thinkercap.next; t != &thinkercap && tele_dest[li] == -1; t = t->next) {
            if (t->function.acp1 != (actionf_p1)P_MobjThinker) continue;
            mobj_t *m = (mobj_t *)t;
            if (m->type == MT_TELEPORTMAN && m->subsector->sector->tag == l->tag) {
                tele_dest[li] = nav_cell(m->x, m->y);
            }
        }
    }
    return tele_dest[li];
}

// Walk the edge a->d. Returns the cell it ends in (a teleporter's
// destination, or the neighbour) or -1, with the cost and any line to use.
static int walk(int a, int d, player_t *p, int *cost, int *use) {
    int bx = a % nav_w + DX[d], by = a / nav_w + DY[d];
    if (bx < 0 || by < 0 || bx >= nav_w || by >= nav_h) return -1;
    int b = by * nav_w + bx, rev;
    if (cflags[b] & C_THING) return -1;
    edge_t *e = edge(a, d, &rev);
    if (e->flags & E_WALL) return -1;
    if (d & 1) {
        // No cutting corners: a diagonal slips past the end of a wall or a
        // standoff that the player's 16-unit radius cannot.
        int c1, u1;
        if (walk(a, d - 1, p, &c1, &u1) < 0 || walk(a, (d + 1) & 7, p, &c1, &u1) < 0) return -1;
    }
    *cost = (d & 1) ? 45 : 32;
    *use = -1;
    short ls[HITS];
    int n = e->n;
    if (e->flags & E_MORE) {
        // Rare: more crossings than the cache holds. Walk them afresh, in
        // the cache's buffer order.
        trace_edge(a, b);
        if (hit_wall) return -1;
        n = nhit;
        rev = 0;
    }
    memcpy(ls, (e->flags & E_MORE) ? hits : e->line, sizeof ls);
    fixed_t ax, ay;
    nav_center(a, &ax, &ay);
    walk_via = -1;
    int last = csec[a];
    for (int i = 0; i < n; i++) {
        int li = ls[rev ? n - 1 - i : i];
        line_t *l = &lines[li];
        int side = P_PointOnLineSide(ax, ay, l);
        if (ap_is_teleport(l->special)) {
            if (side == 0) {
                int t = tele_cell(li);
                if (t >= 0) *cost += 64;
                return t;
            }
            continue;
        }
        // A crossing that does not start in the sector the walk is in is a
        // segment grazing a vertex: the line is touched, not crossed.
        int from = sec_index(side ? l->backsector : l->frontsector);
        if (from != last && sec_index(side ? l->frontsector : l->backsector) == last) continue;
        int u = -1;
        int to = cross(side, li, p, &u, cost);
        if (to < 0) return -1;
        last = to;
        if (l->frontsector != l->backsector && walk_via < 0) walk_via = li;
        if (u != -1 && *use < 0) *use = u;
    }
    if (last != csec[b]) {
        // The segment slipped through a vertex and missed the lines there:
        // only a step the player can take as it stands counts.
        sector_t *S = &sectors[last], *B = &sectors[csec[b]];
        fixed_t top = S->ceilingheight < B->ceilingheight ? S->ceilingheight : B->ceilingheight;
        fixed_t bot = S->floorheight > B->floorheight ? S->floorheight : B->floorheight;
        if (B->floorheight - S->floorheight > STEP || top - bot < HEADROOM) return -1;
    }
    if (!stand(b)) return -1;
    if (tight(b)) *cost += 96;
    int sp = sectors[csec[b]].special;
    if (sp == 4 || sp == 5 || sp == 7 || sp == 11 || sp == 16) *cost += ap_god ? 32 : 400;
    *cost += penalty[b] * 8;
    return b;
}

// ---- search ----

static void hswap(int i, int j) {
    int t = heap[i];
    heap[i] = heap[j];
    heap[j] = t;
    hpos[heap[i]] = i;
    hpos[heap[j]] = j;
}

static void hup(int i) {
    while (i > 0 && dist[heap[(i - 1) / 2]] > dist[heap[i]]) {
        hswap(i, (i - 1) / 2);
        i = (i - 1) / 2;
    }
}

static int hpop(void) {
    int top = heap[0];
    heap[0] = heap[--hlen];
    hpos[heap[0]] = 0;
    for (int i = 0;;) {
        int l = 2 * i + 1, r = l + 1, m = i;
        if (l < hlen && dist[heap[l]] < dist[heap[m]]) m = l;
        if (r < hlen && dist[heap[r]] < dist[heap[m]]) m = r;
        if (m == i) break;
        hswap(i, m);
        i = m;
    }
    hpos[top] = -1;
    return top;
}

void nav_flood(int start, player_t *p) {
    flood_gen++;
    hlen = 0;
    if (start < 0) return;
    gen[start] = flood_gen;
    dist[start] = 0;
    par[start] = -1;
    pdir[start] = -1;
    heap[hlen] = start;
    hpos[start] = hlen++;
    while (hlen) {
        int a = hpop();
        for (int d = 0; d < 8; d++) {
            int cost, use;
            int b = walk(a, d, p, &cost, &use);
            if (b < 0) continue;
            int nd = dist[a] + cost;
            if (gen[b] != flood_gen) {
                gen[b] = flood_gen;
                dist[b] = nd;
                par[b] = a;
                pdir[b] = (signed char)d;
                heap[hlen] = b;
                hpos[b] = hlen++;
                hup(hpos[b]);
            } else if (nd < dist[b] && hpos[b] >= 0) {
                dist[b] = nd;
                par[b] = a;
                pdir[b] = (signed char)d;
                hup(hpos[b]);
            }
        }
    }
}

int nav_dist(int c) { return c >= 0 && gen[c] == flood_gen ? dist[c] : NAV_FAR; }

int nav_path(int goal, int *out, int max) {
    if (nav_dist(goal) == NAV_FAR) return 0;
    int n = 0;
    for (int c = goal; c >= 0; c = par[c]) n++;
    // A path too long to store keeps its start end.
    int k = n;
    for (int c = goal; c >= 0; c = par[c])
        if (--k < max) out[k] = c;
    return n < max ? n : max;
}

int nav_step(int a, int b, player_t *p, int *use, int *via, fixed_t *tx, fixed_t *ty) {
    int d = pdir[b];
    int cost;
    *use = *via = -1;
    if (d < 0 || par[b] != a) {
        for (d = 0; d < 8; d++)
            if (a + DY[d] * nav_w + DX[d] == b) break;
        if (d == 8) return -1;
    }
    int bx = a % nav_w + DX[d], by = a / nav_w + DY[d];
    nav_stand(by * nav_w + bx, tx, ty);
    int r = walk(a, d, p, &cost, use) == b ? 0 : -1;
    *via = walk_via;
    return r;
}

void nav_penalize(int c) {
    if (c >= 0 && penalty[c] < 200) penalty[c] += 50;
}

void nav_decay(void) {
    for (int c = 0; c < ncells; c++)
        if (penalty[c]) penalty[c]--;
}


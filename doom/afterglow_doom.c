// afterglow's side of doomgeneric: a headless platform, an autopilot, and a
// setjmp boundary so the engine's exit paths unwind to the caller instead of
// ending the process.
//
// Copyright (C) 2026 Adam Hitchcock. GPL-2.0-or-later, like the engine it
// links against; see doomgeneric/LICENSE.
//
// build.rs compiles the engine with `exit` defined to `dg_exit`, so every exit
// path reaches the setjmp boundary below.

#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "doomgeneric.h"
#include "d_event.h"
#include "d_player.h"
#include "doomstat.h"
#include "g_game.h"
#include "i_system.h"
#include "i_video.h"
#include "m_menu.h"
#include "m_random.h"
#include "p_local.h"
#include "r_main.h"
#include "st_stuff.h"
#include "tables.h"
#include "w_wad.h"

static jmp_buf guard;
static int guarded, dead;
static uint32_t now_ms;
static int light, started;
extern int dg_viewpct, dg_fov;
extern int dg_palnum;

// Every exit() in the engine (I_Error, I_Quit, -help paths) lands here via the
// -Dexit=dg_exit build flag. Outside a guarded call there is nowhere safe to go back to;
// that cannot happen because the engine only runs inside dgx_* calls.
void exit(int code) {
    (void)code;
    dead = 1;
    if (guarded) {
        longjmp(guard, 1);
    }
    abort();
}

void DG_Init(void) {}
void DG_DrawFrame(void) {}
void DG_SleepMs(uint32_t ms) { now_ms += ms; }
uint32_t DG_GetTicksMs(void) { return now_ms; }
int DG_GetKey(int *pressed, unsigned char *key) {
    (void)pressed;
    (void)key;
    return 0;
}
void DG_SetWindowTitle(const char *title) { (void)title; }

// ---- autopilot ----

#define VCELL 64
#define VDIM 256
static unsigned char visited[VDIM][VDIM];
static int stuck, still, wander, use_tic, bored, offlevel;
static fixed_t lastx, lasty;
static angle_t goal;
static uint32_t rng = 1;

static uint32_t next(void) {
    rng ^= rng << 13;
    rng ^= rng >> 17;
    rng ^= rng << 5;
    return rng;
}

static unsigned char *vcell(fixed_t x, fixed_t y) {
    int cx = ((x >> FRACBITS) / VCELL + VDIM / 2) & (VDIM - 1);
    int cy = ((y >> FRACBITS) / VCELL + VDIM / 2) & (VDIM - 1);
    return &visited[cx][cy];
}

static int is_mobj(thinker_t *t) {
    return t->function.acp1 == (actionf_p1)P_MobjThinker;
}

static mobj_t *nearest_visible(mobj_t *me) {
    mobj_t *best = NULL;
    fixed_t bd = 1500 * FRACUNIT;
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

// Clear 48-unit steps straight ahead along a, up to 8.
static int clear_steps(mobj_t *me, angle_t a, fixed_t *ex, fixed_t *ey) {
    int fine = a >> ANGLETOFINESHIFT, ok = 0;
    *ex = me->x;
    *ey = me->y;
    for (int step = 1; step <= 8; step++) {
        fixed_t nx = me->x + FixedMul(48 * step * FRACUNIT, finecosine[fine]);
        fixed_t ny = me->y + FixedMul(48 * step * FRACUNIT, finesine[fine]);
        if (!P_CheckPosition(me, nx, ny)) break;
        if (tmfloorz - me->z > 24 * FRACUNIT || tmceilingz - tmfloorz < 56 * FRACUNIT) break;
        ok = step;
        *ex = nx;
        *ey = ny;
    }
    return ok;
}

// How good a heading is. Best is open floor across a narrow cone ahead: one
// straight ray lets the player slide along a wall with its nose to it, which
// on the panel is a frozen grey screen. Then, for corridors too narrow for the
// cone, open floor straight ahead; then whatever is least blocked. Within a
// tier, more floor and less often walked wins.
static int score_dir(mobj_t *me, angle_t a) {
    fixed_t x, y, sx, sy;
    int ok = clear_steps(me, a, &x, &y);
    int left = clear_steps(me, a + ANG45 / 2, &sx, &sy);
    int right = clear_steps(me, a - ANG45 / 2, &sx, &sy);
    if (ok < 2) return ok - 1000;
    int seen = *vcell(x, y);
    int s = ok * 10 - (seen > 40 ? 40 : seen) * 2 + (int)(next() & 7);
    return left >= 2 && right >= 2 ? s + 1000 : s;
}

// A pit or dead end the heuristics cannot leave: cut to a random item, which
// map authors only place where a player can stand.
static void hop(mobj_t *me) {
    int n = 0;
    for (thinker_t *t = thinkercap.next; t != &thinkercap; t = t->next)
        if (is_mobj(t) && (((mobj_t *)t)->flags & MF_SPECIAL)) n++;
    if (!n) return;
    int k = (int)(next() % (uint32_t)n);
    for (thinker_t *t = thinkercap.next; t != &thinkercap; t = t->next) {
        if (!is_mobj(t) || !(((mobj_t *)t)->flags & MF_SPECIAL)) continue;
        if (k-- == 0) {
            mobj_t *m = (mobj_t *)t;
            P_TeleportMove(me, m->x, m->y);
            me->z = me->floorz;
            return;
        }
    }
}

static int pick_map(int *episode, int *map);

// Called from the end of G_BuildTiccmd (one-line patch in g_game.c).
void DG_Autopilot(ticcmd_t *cmd) {
    if (gamestate != GS_LEVEL) {
        // Intermission and text screens wait for a button; the finale after
        // an episode's last map never ends, so give up on it and warp.
        if (++offlevel % 35 == 0) cmd->buttons |= BT_USE;
        if (offlevel > 35 * 15) {
            int e, m;
            if (pick_map(&e, &m)) G_DeferedInitNew(gameskill, e, m);
            offlevel = 0;
        }
        return;
    }
    offlevel = 0;
    player_t *p = &players[consoleplayer];
    mobj_t *me = p->mo;
    if (!me) return;
    p->cheats |= CF_GODMODE;
    p->health = me->health = 100;
    p->extralight = light;
    for (int w = 0; w < NUMWEAPONS; w++) p->weaponowned[w] = true;
    for (int a = 0; a < NUMAMMO; a++) p->ammo[a] = p->maxammo[a];
    if (p->readyweapon < wp_shotgun && p->pendingweapon == wp_nochange) p->pendingweapon = wp_chaingun;

    unsigned char *vc = vcell(me->x, me->y);
    bored = *vc == 0 ? 0 : bored + 1;
    if (*vc < 250) (*vc)++;
    // Standing still for 2 s means boxed in (a start closet, a pit, a
    // monster in a doorway): nothing on screen moves, so cut away sooner.
    still = P_AproxDistance(me->x - lastx, me->y - lasty) < 2 * FRACUNIT ? still + 1 : 0;
    if (bored > 35 * 8 || still > 35 * 2) {
        hop(me);
        bored = still = 0;
    }

    mobj_t *t = nearest_visible(me);
    angle_t want;
    if (t) {
        want = R_PointToAngle2(me->x, me->y, t->x, t->y);
        cmd->buttons |= BT_ATTACK;
        cmd->forwardmove = 10;
    } else {
        fixed_t moved = P_AproxDistance(me->x - lastx, me->y - lasty);
        stuck = moved < 2 * FRACUNIT ? stuck + 1 : 0;
        // Facing is only judged once a turn has landed; mid-turn it is
        // always somewhere the autopilot already chose to leave.
        int turned = (int)(goal - me->angle) >> 16;
        int nosed = turned > -1024 && turned < 1024 && score_dir(me, me->angle) < -900;
        if (stuck > 6 || --wander <= 0 || nosed) {
            int best = -100000;
            angle_t ba = goal;
            for (int i = 0; i < 16; i++) {
                angle_t a = me->angle + (angle_t)i * (ANG90 / 4);
                int s = score_dir(me, a) - (i == 8 ? 4 : 0);
                if (s > best) {
                    best = s;
                    ba = a;
                }
            }
            goal = ba;
            wander = 20 + (int)(next() & 31);
            stuck = 0;
        }
        want = goal;
        cmd->forwardmove = 40;
    }
    lastx = me->x;
    lasty = me->y;
    int diff = (int)(want - me->angle) >> 16;
    if (diff > 1280) diff = 1280;
    if (diff < -1280) diff = -1280;
    cmd->angleturn = (short)diff;
    if (++use_tic % 10 == 0) cmd->buttons |= BT_USE;
}

// Maps the loaded IWAD actually has, so Freedoom, shareware and DOOM2-style
// WADs all work.
static int pick_map(int *episode, int *map) {
    char name[9];
    int have[36][2], n = 0;
    if (gamemode == commercial) {
        for (int m = 1; m <= 32; m++) {
            snprintf(name, sizeof name, "MAP%02d", m);
            if (W_CheckNumForName(name) >= 0) {
                have[n][0] = 1;
                have[n++][1] = m;
            }
        }
    } else {
        for (int e = 1; e <= 4; e++)
            for (int m = 1; m <= 9; m++) {
                snprintf(name, sizeof name, "E%dM%d", e, m);
                if (W_CheckNumForName(name) >= 0) {
                    have[n][0] = e;
                    have[n++][1] = m;
                }
            }
    }
    if (!n) return 0;
    int k = (int)(next() % (uint32_t)n);
    *episode = have[k][0];
    *map = have[k][1];
    return 1;
}

static void reset_autopilot(void) {
    memset(visited, 0, sizeof visited);
    stuck = still = wander = bored = offlevel = 0;
}

// ---- the API afterglow calls; every entry is guarded ----

#define GUARD(fail)                 \
    if (dead) return fail;          \
    if (setjmp(guard)) {            \
        guarded = 0;                \
        return fail;                \
    }                               \
    guarded = 1

int dgx_init(const char *wad, uint32_t seed) {
    static char *argv[] = {"doom", "-iwad", NULL, "-config", "/dev/null", "-extraconfig", "/dev/null", "-skill", "3", NULL};
    GUARD(-1);
    rng = seed | 1;
    argv[2] = (char *)wad;
    doomgeneric_Create(9, argv);
    started = 1;
    guarded = 0;
    return 0;
}

// The screen is `width` x 200, the 3D view `pct` of that width with its
// horizontal field of view `fov` degrees (0: Hor+). Before init or after.
void dgx_view(int width, int pct, int fov) {
    dg_screenwidth = width < ORIGWIDTH ? ORIGWIDTH : width > MAXSCREENWIDTH ? MAXSCREENWIDTH : width;
    dg_viewpct = pct;
    dg_fov = fov;
    if (started && !dead) {
        R_SetViewSize(screenblocks, detailLevel);
        ST_Invalidate();
    }
}

// Returns the map as episode * 100 + map, or -1 if the engine has died.
// `brightness` is the player's extralight, 0..2: Doom's own gun-flash boost,
// held on so dark sectors do not read as a black panel.
int dgx_warp(uint32_t seed, int brightness) {
    int e = 1, m = 1;
    GUARD(-1);
    rng ^= seed | 1;
    light = brightness;
    if (pick_map(&e, &m)) G_DeferedInitNew(sk_hard, e, m);
    reset_autopilot();
    guarded = 0;
    return e * 100 + m;
}

int dgx_tick(uint32_t ms) {
    GUARD(-1);
    now_ms += ms;
    doomgeneric_Tick();
    guarded = 0;
    return 0;
}

// The last drawn frame, SCREENWIDTH x 200, its width, and which PLAYPAL
// palette it is shown with.
const uint8_t *dgx_frame(int *width, int *palette) {
    *width = SCREENWIDTH;
    *palette = dg_palnum;
    return I_VideoBuffer;
}

int dgx_fault(void) {
    GUARD(-1);
    I_Error("dgx_fault: deliberate engine error");
    guarded = 0;
    return 0;
}

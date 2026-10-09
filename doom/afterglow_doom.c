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
// Doom's skill_t (0 = I'm Too Young To Die .. 4 = Nightmare), for the next
// map; and whether the player is invulnerable.
static int skill = sk_medium, god = 1;
static int dead_tics;
extern int dg_viewpct, dg_fov, dg_hud;
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

extern int ap_god;
void AP_Tick(ticcmd_t *cmd);
static int offlevel;
static uint32_t rng = 1;

static uint32_t next(void) {
    rng ^= rng << 13;
    rng ^= rng >> 17;
    rng ^= rng << 5;
    return rng;
}

static int pick_map(int *episode, int *map);

// Called from the end of G_BuildTiccmd (one-line patch in g_game.c). The
// player itself is doom/autopilot.c; this keeps the screens between maps
// moving, a death moving on to a new map, and the arsenal full.
void DG_Autopilot(ticcmd_t *cmd) {
    if (gamestate != GS_LEVEL) {
        // A finished map shows its tally for 5 s, then a new random map
        // starts. Text screens wait for a button; the finale after an
        // episode's last map never ends, so give up on it too.
        if (++offlevel % 35 == 0) cmd->buttons |= BT_USE;
        if (offlevel > 35 * (gamestate == GS_INTERMISSION ? 5 : 15)) {
            int e, m;
            if (pick_map(&e, &m)) G_DeferedInitNew(skill, e, m);
            offlevel = 0;
        }
        return;
    }
    offlevel = 0;
    player_t *p = &players[consoleplayer];
    mobj_t *me = p->mo;
    if (!me) return;
    // Dead: hold on the death view for 3 s, then a new map. Doom itself would
    // wait for a use press and replay the same map.
    if (p->playerstate == PST_DEAD) {
        if (++dead_tics > 35 * 3) {
            int e, m;
            if (pick_map(&e, &m)) G_DeferedInitNew(skill, e, m);
            dead_tics = 0;
        }
        return;
    }
    dead_tics = 0;
    ap_god = god;
    if (god) {
        p->cheats |= CF_GODMODE;
        p->health = me->health = 100;
    } else {
        p->cheats &= ~CF_GODMODE;
    }
    p->extralight = light;
    for (int w = 0; w < NUMWEAPONS; w++) p->weaponowned[w] = true;
    for (int a = 0; a < NUMAMMO; a++) p->ammo[a] = p->maxammo[a];
    AP_Tick(cmd);
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

// ---- the API afterglow calls; every entry is guarded ----

#define GUARD(fail)                 \
    if (dead) return fail;          \
    if (setjmp(guard)) {            \
        guarded = 0;                \
        return fail;                \
    }                               \
    guarded = 1

// Full height: the view runs to the bottom of the screen and the status bar,
// if any, is drawn over it (see dg_hud in d_main.c).
static void apply_view(void) {
    screenblocks = 11;
    R_SetViewSize(screenblocks, detailLevel);
    ST_Invalidate();
}

int dgx_init(const char *wad, uint32_t seed) {
    static char *argv[] = {"doom", "-iwad", NULL, "-config", "/dev/null", "-extraconfig", "/dev/null", "-skill", "3", NULL};
    GUARD(-1);
    rng = seed | 1;
    argv[2] = (char *)wad;
    doomgeneric_Create(9, argv);
    started = 1;
    apply_view();
    guarded = 0;
    return 0;
}

// The screen is `width` x 200, the 3D view `pct` of that width with its
// horizontal field of view `fov` degrees (0: Hor+), and `hud` 1 to overlay
// the status bar. `skill_1to5` takes effect on the next map, `godmode` at
// once. Before init or after.
void dgx_view(int width, int pct, int fov, int hud, int skill_1to5, int godmode) {
    skill = skill_1to5 < 1 ? 0 : skill_1to5 > 5 ? 4 : skill_1to5 - 1;
    god = godmode;
    dg_screenwidth = width < ORIGWIDTH ? ORIGWIDTH : width > MAXSCREENWIDTH ? MAXSCREENWIDTH : width;
    dg_viewpct = pct;
    dg_fov = fov;
    dg_hud = hud;
    if (started && !dead) apply_view();
}

// Returns the map as episode * 100 + map, or -1 if the engine has died.
// `brightness` is the player's extralight, 0..2: Doom's own gun-flash boost,
// held on so dark sectors do not read as a black panel.
int dgx_warp(uint32_t seed, int brightness) {
    int e = 1, m = 1;
    GUARD(-1);
    rng ^= seed | 1;
    light = brightness;
    if (pick_map(&e, &m)) G_DeferedInitNew(skill, e, m);
    offlevel = dead_tics = 0;
    guarded = 0;
    return e * 100 + m;
}

// Returns a count of the levels started so far, so the host can time each
// map from its start, however it began: a warp, an exit or a death.
int dgx_tick(uint32_t ms) {
    static int serial, ep, map, time;
    GUARD(-1);
    now_ms += ms;
    doomgeneric_Tick();
    if (gamestate == GS_LEVEL && (gameepisode != ep || gamemap != map || leveltime < time)) serial = (serial + 1) & 0x3fffffff;
    if (gamestate == GS_LEVEL) ep = gameepisode, map = gamemap, time = leveltime;
    guarded = 0;
    return serial;
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

// Headless autopilot benchmark: plays each map at full speed and reports
// whether the autopilot reached the exit, how long it took, and kills and
// secrets. `mise run bench-doom [maps...]` builds and runs it.
//
// Copyright (C) 2026 Adam Hitchcock. GPL-2.0-or-later, like the engine.
//
// Usage: bench WAD MINUTES SKILL GOD [ExMy... | all] [-frames DIR] [-seed N]

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "doomstat.h"
#include "d_main.h"
#include "d_player.h"
#include "g_game.h"
#include "i_video.h"
#include "w_wad.h"
#include "z_zone.h"

int dgx_init(const char *wad, uint32_t seed);
int dgx_tick(uint32_t ms);
void dgx_view(int width, int pct, int fov, int hud, int skill_1to5, int godmode);
extern uint32_t ap_seed;
extern int ap_hops, ap_evals;
extern double ap_eval_ms, ap_eval_max_ms;
extern int prndindex;

static void dump(const char *dir, const char *map, int n) {
    char path[512];
    snprintf(path, sizeof path, "%s/%s-%03d.ppm", dir, map, n);
    FILE *f = fopen(path, "wb");
    if (!f) return;
    const byte *pal = W_CacheLumpName("PLAYPAL", PU_CACHE);
    fprintf(f, "P6\n%d %d\n255\n", SCREENWIDTH, SCREENHEIGHT);
    for (int i = 0; i < SCREENWIDTH * SCREENHEIGHT; i++) fwrite(pal + I_VideoBuffer[i] * 3, 1, 3, f);
    fclose(f);
}

static void run(int e, int m, int minutes, int skill, const char *frames) {
    char name[9];
    if (gamemode == commercial) snprintf(name, sizeof name, "MAP%02d", m);
    else snprintf(name, sizeof name, "E%dM%d", e, m);
    if (W_CheckNumForName(name) < 0) return;
    G_DeferedInitNew(skill, e, m);
    int hops0 = ap_hops;
    ap_evals = 0;
    ap_eval_ms = ap_eval_max_ms = 0;
    int limit = minutes * 60 * 35, started = 0, deaths = 0, shots = 0, played = 0, last = -1;
    for (int i = 0; i < limit * 4 + 35 * 30; i++) {
        if (dgx_tick(29) < 0) {
            printf("%-6s engine died\n", name);
            exit(1);
        }
        if (!started && gamestate == GS_LEVEL && gamemap == m && (gamemode == commercial || gameepisode == e)) {
            started = 1;
            prndindex = (int)(ap_seed * 37 & 0xff);  // a different game per seed
        }
        if (!started) continue;
        if (gamestate == GS_LEVEL && leveltime != last) played++, last = leveltime;
        player_t *p = &players[consoleplayer];
        // A death ends the map: the saver moves on to a new one.
        deaths = p->playerstate == PST_DEAD;
        if (frames && gamestate == GS_LEVEL && played % (35 * 10) == 0 && played / 350 >= shots)
            dump(frames, name, shots++);
        int exited = gamestate == GS_INTERMISSION || gamestate == GS_FINALE || gameaction == ga_completed || gameaction == ga_victory;
        if (exited || deaths || played >= limit || gamestate != GS_LEVEL) {
            printf("%-6s %-4s %6.1f  kills %3d/%-3d  secrets %2d/%-2d  deaths %d  hops %d  evals %d  eval-ms %.1f max %.2f\n",
                   name, exited ? "EXIT" : deaths ? "DIED" : "--", played / 35.0, p->killcount, totalkills, p->secretcount,
                   totalsecret, deaths, ap_hops - hops0, ap_evals, ap_eval_ms, ap_eval_max_ms);
            fflush(stdout);
            return;
        }
    }
    printf("%-6s stalled\n", name);
}

int main(int argc, char **argv) {
    if (argc < 5) {
        fprintf(stderr, "usage: bench WAD MINUTES SKILL GOD [ExMy... | all] [-frames DIR]\n");
        return 2;
    }
    const char *frames = NULL;
    for (int i = 5; i + 1 < argc; i++) {
        if (!strcmp(argv[i], "-frames")) frames = argv[i + 1];
        if (!strcmp(argv[i], "-seed")) ap_seed = (uint32_t)atoi(argv[i + 1]);
    }
    int minutes = atoi(argv[2]), skill = atoi(argv[3]) - 1;
    dgx_view(320, 100, 0, 1, skill + 1, atoi(argv[4]));
    if (dgx_init(argv[1], 7) < 0) return 1;
    int any = 0;
    for (int i = 5; i < argc; i++) {
        if (!strcmp(argv[i], "-frames") || !strcmp(argv[i], "-seed")) {
            i++;
            continue;
        }
        int e = 1, m = 1;
        if (!strcmp(argv[i], "all")) {
            for (e = 1; e <= 4; e++)
                for (m = 1; m <= 9; m++) run(e, m, minutes, skill, frames);
            for (m = 1; m <= 32 && gamemode == commercial; m++) run(1, m, minutes, skill, frames);
        } else if (sscanf(argv[i], "E%dM%d", &e, &m) == 2 || sscanf(argv[i], "MAP%d", &m) == 1) {
            run(e, m, minutes, skill, frames);
        }
        any = 1;
    }
    if (!any)
        for (int m = 1; m <= 9; m++) run(1, m, minutes, skill, frames);
    return 0;
}

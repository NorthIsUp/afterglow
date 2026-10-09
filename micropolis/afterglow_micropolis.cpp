/* afterglow's glue to the Micropolis engine: a C API over one simulator, and
 * the setjmp boundary every engine call runs inside.
 *
 * Exactly one thread (src/micropolis/engine.rs) calls in. An engine fatal
 * error (assert, NOT_REACHED) longjmps back here and the call returns -1.
 * The simulator is then abandoned, never touched again: unwinding skipped its
 * destructors and its invariants are whatever the failed tick left.
 *
 * Copyright (C) 2026 afterglow contributors. Links against the Micropolis
 * engine, so GPL-3.0-or-later with EA's additional terms (engine/micropolis.h).
 */

#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "engine/micropolis.h"

/* Private state the C API reports, named once. */
struct AfterglowAccess {
    static const unsigned short *map(const Micropolis *m) { return m->mapBase; }
    static void valves(const Micropolis *m, int *r, int *c, int *i)
    {
        *r = m->resValve;
        *c = m->comValve;
        *i = m->indValve;
    }
    static void money(const Micropolis *m, int *land, int *flow, int *roads)
    {
        *land = m->landValueAverage;
        *flow = m->cashFlow;
        *roads = m->roadTotal;
    }
};

static Micropolis *sim;
static jmp_buf boundary;
static bool guarded;
static const unsigned char *city_bytes;
static size_t city_len;

void afterglow_fatal(const char *what, const char *file, int line)
{
    fprintf(stderr, "[screensaver] micropolis: %s at %s:%d\n", what, file, line);
    if (guarded) {
        longjmp(boundary, 1);
    }
    abort();
}

FILE *afterglow_fopen(const char *name, const char *mode)
{
    if (strcmp(name, "afterglow:city") == 0) {
        return fmemopen((void *)city_bytes, city_len, mode);
    }
    return fopen(name, mode);
}

#define GUARD(fail)                      \
    if (sim == NULL) {                   \
        return fail;                     \
    }                                    \
    guarded = true;                      \
    if (setjmp(boundary) != 0) {         \
        guarded = false;                 \
        sim = NULL;                      \
        return fail;                     \
    }
#define UNGUARD() guarded = false

extern "C" {

struct MpxStats {
    int pop;
    int year;
    int month;
    int funds;
    int res_valve;
    int com_valve;
    int ind_valve;
    int res_cap;
    int com_cap;
    int ind_cap;
    int powered;
    int unpowered;
    int coal;
    int nuclear;
    int police;
    int fire;
    int stadium;
    int seaport;
    int airport;
    int tax;
    int crime;
    int pollution;
    int res_pop;
    int com_pop;
    int ind_pop;
    int city_time;
    int score;
    int land_value;
    int cash_flow;
    int roads;
};

int mpx_init(void)
{
    guarded = true;
    if (setjmp(boundary) != 0) {
        guarded = false;
        sim = NULL;
        return -1;
    }
    sim = new Micropolis();
    sim->initGame();
    sim->setEnableDisasters(false);
    sim->setAutoBudget(true);
    sim->setAutoBulldoze(true);
    sim->setAutoGoto(false);
    sim->setEnableSound(false);
    UNGUARD();
    return 0;
}

/* A fresh city on new terrain, or bundled city `bytes` when non-NULL. */
int mpx_new_city(int seed, const unsigned char *bytes, int len, int funds)
{
    GUARD(-1);
    bool ok = true;
    if (bytes != NULL) {
        city_bytes = bytes;
        city_len = (size_t)len;
        ok = sim->loadCity("afterglow:city");
        city_bytes = NULL;
    } else {
        sim->generateSomeCity(seed);
        sim->setFunds(funds);
        sim->setCityTax(7);
    }
    sim->setEnableDisasters(false);
    sim->setAutoBudget(true);
    sim->setAutoBulldoze(true);
    sim->setPasses(1);
    sim->setSpeed(3);
    UNGUARD();
    return ok ? 0 : 1;
}

/* `ticks` simulator passes, each a sixteenth of a city week. */
int mpx_step(int ticks)
{
    GUARD(-1);
    for (int i = 0; i < ticks; i++) {
        sim->simTick();
    }
    UNGUARD();
    return 0;
}

int mpx_animate(void)
{
    GUARD(-1);
    sim->animateTiles();
    UNGUARD();
    return 0;
}

/* A ToolResult (1 is done), or -100 if the engine died. */
int mpx_tool(int tool, int x, int y)
{
    GUARD(-100);
    int r = sim->doTool((EditingTool)tool, (short)x, (short)y);
    UNGUARD();
    return r;
}

int mpx_disaster(int kind)
{
    GUARD(-1);
    switch (kind) {
    case 0: sim->makeFire(); break;
    case 1: sim->makeFlood(); break;
    case 2: sim->makeEarthquake(); break;
    case 3: sim->makeMeltdown(); break;
    default: break;
    }
    UNGUARD();
    return 0;
}

int mpx_set_tax(int tax)
{
    GUARD(-1);
    sim->setCityTax((short)tax);
    UNGUARD();
    return 0;
}

/* Column-major, x * 100 + y; NULL once the engine died. */
const unsigned short *mpx_map(void)
{
    return sim == NULL ? NULL : AfterglowAccess::map(sim);
}

int mpx_stats(MpxStats *s)
{
    if (sim == NULL) {
        return -1;
    }
    s->pop = (int)sim->cityPop;
    s->year = (int)(sim->cityTime / 48 + sim->startingYear);
    s->month = (int)((sim->cityTime % 48) >> 2);
    s->funds = (int)sim->totalFunds;
    AfterglowAccess::valves(sim, &s->res_valve, &s->com_valve, &s->ind_valve);
    s->res_cap = sim->resCap;
    s->com_cap = sim->comCap;
    s->ind_cap = sim->indCap;
    s->powered = sim->poweredZoneCount;
    s->unpowered = sim->unpoweredZoneCount;
    s->coal = sim->coalPowerPop;
    s->nuclear = sim->nuclearPowerPop;
    s->police = sim->policeStationPop;
    s->fire = sim->fireStationPop;
    s->stadium = sim->stadiumPop;
    s->seaport = sim->seaportPop;
    s->airport = sim->airportPop;
    s->tax = sim->cityTax;
    s->crime = sim->crimeAverage;
    s->pollution = sim->pollutionAverage;
    s->res_pop = sim->resPop;
    s->com_pop = sim->comPop;
    s->ind_pop = sim->indPop;
    s->city_time = (int)sim->cityTime;
    s->score = sim->cityScore;
    AfterglowAccess::money(sim, &s->land_value, &s->cash_flow, &s->roads);
    return 0;
}

/* Fail the next call through the boundary, for the crash-safety test. */
int mpx_fault(void)
{
    GUARD(-1);
    afterglow_fatal("fault injected", __FILE__, __LINE__);
}

}

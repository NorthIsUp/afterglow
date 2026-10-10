// The C side of the `gba` saver: one mGBA core behind a handful of calls,
// with a setjmp boundary around everything that runs core code. build.rs
// defines `abort` to `agb_abort`, and a SIGSEGV or SIGBUS on the engine
// thread inside the boundary jumps back too, so a broken core fails the call
// instead of ending the process.

#include <pthread.h>
#include <setjmp.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include <mgba/core/core.h>
#include <mgba/core/log.h>
#include <mgba/gba/core.h>
#include <mgba-util/vfs.h>

#define AGB_W 240
#define AGB_H 160

struct agb {
	struct mCore* core;
	color_t pix[AGB_W * AGB_H];
};

static _Thread_local jmp_buf* boundary;

void agb_abort(void) {
	if (boundary) {
		longjmp(*boundary, 1);
	}
	_Exit(134);
}

static struct sigaction prev_segv, prev_bus;

// Anything outside the boundary goes to the handler that was there (Rust's
// stack-overflow report, or the default), which ends the process as before.
static void fault(int sig, siginfo_t* info, void* ctx) {
	if (boundary) {
		longjmp(*boundary, 1);
	}
	struct sigaction* prev = sig == SIGSEGV ? &prev_segv : &prev_bus;
	if (prev->sa_flags & SA_SIGINFO) {
		prev->sa_sigaction(sig, info, ctx);
	} else if (prev->sa_handler != SIG_DFL && prev->sa_handler != SIG_IGN) {
		prev->sa_handler(sig);
	} else {
		signal(sig, SIG_DFL);
	}
}

// Late, from the first `agb_new`, so std has already installed its handler
// to chain to. SA_NODEFER leaves the signal unblocked after the longjmp;
// SA_ONSTACK runs on std's alternate stack when the fault is an overflow.
static void install_fault_handler(void) {
	struct sigaction sa = { 0 };
	sa.sa_sigaction = fault;
	sa.sa_flags = SA_SIGINFO | SA_NODEFER | SA_ONSTACK;
	sigemptyset(&sa.sa_mask);
	sigaction(SIGSEGV, &sa, &prev_segv);
	sigaction(SIGBUS, &sa, &prev_bus);
}

static void quiet(struct mLogger* l, int category, enum mLogLevel level, const char* fmt, va_list args) {
	(void) l;
	(void) category;
	(void) level;
	(void) fmt;
	(void) args;
}

static struct mLogger silent = { .log = quiet };

// Every entry point that runs core code arms `boundary` first; an abort
// unwinds to it, and the machine is never touched again but to free it.
#define ARM(fail)                 \
	jmp_buf jb;                   \
	jmp_buf* prev = boundary;     \
	if (setjmp(jb)) {             \
		boundary = prev;          \
		return fail;              \
	}                             \
	boundary = &jb
#define DISARM() boundary = prev

static struct mCore* boot(color_t* pix, const uint8_t* rom, size_t rom_len, const uint8_t* bios, size_t bios_len) {
	struct mCore* core = GBACoreCreate();
	if (!core) {
		return NULL;
	}
	if (!core->init(core)) {
		free(core);
		return NULL;
	}
	mCoreInitConfig(core, NULL);
	core->setVideoBuffer(core, pix, AGB_W);
	// No battery file: savedata lives in memory and is never written out.
	if (!core->loadROM(core, VFileFromConstMemory(rom, rom_len))) {
		core->deinit(core);
		return NULL;
	}
	if (bios && bios_len) {
		core->loadBIOS(core, VFileMemChunk(bios, bios_len), 0);
	}
	core->reset(core);
	return core;
}

// NULL if the core rejects the ROM. The ROM is borrowed, not copied (a 32 MiB
// cartridge twice over would not fit the pod), so it must outlive the
// machine. A NULL `bios` runs mGBA's own HLE BIOS.
struct agb* agb_new(const uint8_t* rom, size_t rom_len, const uint8_t* bios, size_t bios_len) {
	static pthread_once_t once = PTHREAD_ONCE_INIT;
	pthread_once(&once, install_fault_handler);
	mLogSetDefaultLogger(&silent);
	struct agb* a = calloc(1, sizeof(*a));
	if (!a) {
		return NULL;
	}
	// shortcut: an abort mid-boot leaks the half-built core; it has never
	// happened, and a ROM that does it is skipped for good.
	ARM(NULL);
	a->core = boot(a->pix, rom, rom_len, bios, bios_len);
	DISARM();
	if (!a->core) {
		free(a);
		return NULL;
	}
	return a;
}

// One frame with `keys` held (GBA KEYINPUT bit order, 1 = pressed). 0, or -1
// if the core aborted and this machine must not be run again.
int agb_frame(struct agb* a, uint32_t keys) {
	ARM(-1);
	a->core->setKeys(a->core, keys);
	a->core->runFrame(a->core);
	DISARM();
	return 0;
}

// The last frame, 240x160 mGBA colour (XBGR8888, red in the low byte).
const uint32_t* agb_pixels(const struct agb* a) {
	return (const uint32_t*) a->pix;
}

// A byte as the CPU would read it, without side effects on I/O.
uint8_t agb_read8(struct agb* a, uint32_t addr) {
	ARM(0);
	uint8_t v = (uint8_t) a->core->rawRead8(a->core, addr, -1);
	DISARM();
	return v;
}

void agb_free(struct agb* a) {
	if (!a) {
		return;
	}
	if (a->core) {
		ARM();
		a->core->deinit(a->core);
		DISARM();
	}
	free(a);
}

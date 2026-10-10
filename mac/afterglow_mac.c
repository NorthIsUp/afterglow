// afterglow's side of Mini vMac: a headless platform layer in place of the
// emulator's OSGLU*.c, with every tick handed to Rust (`afg_mac_tick` in
// engine.rs). The crash boundary is the process: this runs in `mac-engine`,
// never in the screensaver.
//
// Copyright (C) 2026 Adam Hitchcock. GPL-2.0, like the emulator it links
// against; see minivmac/COPYING.txt. Modelled on minivmac/OSGLUSDL.c.

#include "CNFGRAPI.h"
#include "SYSDEPNS.h"
#include "ENDIANAC.h"
#include "MYOSGLUE.h"
#include "STRCONST.h"

GLOBALOSGLUPROC MyMoveBytes(anyp srcPtr, anyp destPtr, si5b byteCount) {
    (void)memcpy((char *)destPtr, (char *)srcPtr, byteCount);
}

#include "COMOSGLU.h"
#include "PBUFSTDC.h"
#include "PROGMAIN.h"

// Rust, once per emulated sixtieth: hands the screen (512x342, 1 bit per
// pixel, MSB first, 1 = black) on and may call mvx_mouse / mvx_key /
// mvx_speed. Returns 0 to run on, 1 to reset the Mac, 2 to stop.
extern int afg_mac_tick(const unsigned char *screen);

static int booted;
static struct timespec tick_start;

// Seconds from 1904 (the Mac's epoch) to 1970.
#define MAC_EPOCH_OFFSET 2082844800u

// ---- drives: read from the user's files on demand and never written to
// them; the Mac's writes land in 512-byte blocks in memory, dropped on every
// reset so each boot starts from the files as they are ----

#define BLOCK 512

static FILE *Drives[NumDrives];
static ui5r DiskSize[NumDrives];
static unsigned char **Written[NumDrives];

static void forget_writes(int i) {
    ui5r blocks = (DiskSize[i] + BLOCK - 1) / BLOCK;
    for (ui5r b = 0; b < blocks; ++b) {
        free(Written[i][b]);
        Written[i][b] = NULL;
    }
}

static int read_file(int i, ui5r at, unsigned char *out, ui5r n) {
    return 0 == fseek(Drives[i], (long)at, SEEK_SET) && fread(out, 1, n, Drives[i]) == n;
}

GLOBALOSGLUFUNC tMacErr vSonyTransfer(blnr IsWrite, ui3p Buffer, tDrive Drive_No, ui5r Sony_Start,
                                      ui5r Sony_Count, ui5r *Sony_ActCount) {
    ui5r size = DiskSize[Drive_No], done = 0;
    ui5r n = Sony_Start >= size ? 0 : size - Sony_Start < Sony_Count ? size - Sony_Start : Sony_Count;
    tMacErr err = n == Sony_Count ? mnvm_noErr : mnvm_eofErr;
    while (done < n) {
        ui5r at = Sony_Start + done, b = at / BLOCK, off = at % BLOCK;
        ui5r len = BLOCK - off < n - done ? BLOCK - off : n - done;
        unsigned char **blk = &Written[Drive_No][b];
        if (IsWrite) {
            if (!*blk) {
                ui5r base = b * BLOCK, have = size - base < BLOCK ? size - base : BLOCK;
                *blk = calloc(1, BLOCK);
                if (!*blk || !read_file(Drive_No, base, *blk, have)) {
                    err = mnvm_miscErr;
                    break;
                }
            }
            memcpy(*blk + off, Buffer + done, len);
        } else if (*blk) {
            memcpy(Buffer + done, *blk + off, len);
        } else if (!read_file(Drive_No, at, Buffer + done, len)) {
            err = mnvm_miscErr;
            break;
        }
        done += len;
    }
    if (nullpr != Sony_ActCount) *Sony_ActCount = done;
    return err;
}

GLOBALOSGLUFUNC tMacErr vSonyGetSize(tDrive Drive_No, ui5r *Sony_Count) {
    *Sony_Count = DiskSize[Drive_No];
    return mnvm_noErr;
}

// The Mac ejects a disk on shutdown or from the Finder; the file stays open
// for the next boot, which re-inserts it.
GLOBALOSGLUFUNC tMacErr vSonyEject(tDrive Drive_No) {
    DiskEjectedNotify(Drive_No);
    return mnvm_noErr;
}

static int open_disk(int i, const char *path) {
    long size;
    Drives[i] = fopen(path, "rb");
    if (!Drives[i] || 0 != fseek(Drives[i], 0, SEEK_END) || (size = ftell(Drives[i])) <= 0) return 0;
    DiskSize[i] = (ui5r)size;
    Written[i] = calloc((DiskSize[i] + BLOCK - 1) / BLOCK, sizeof *Written[i]);
    return NULL != Written[i];
}

static void insert_all(void) {
    for (tDrive i = 0; i < NumDrives; ++i) {
        if (Drives[i] && !vSonyIsInserted(i)) DiskInsertNotify(i, falseblnr);
    }
}

// ---- time ----

static void stamp(void) {
    CurMacDateInSeconds = (ui5b)time(NULL) + MAC_EPOCH_OFFSET;
}

// Speeds above 1x run extra emulation until 12 ms of the 16.6 ms sixtieth are
// gone, leaving the rest to the render thread.
GLOBALOSGLUFUNC blnr ExtraTimeNotOver(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    long ns = (now.tv_sec - tick_start.tv_sec) * 1000000000L + (now.tv_nsec - tick_start.tv_nsec);
    return ns < 12000000L;
}

GLOBALOSGLUPROC DoneWithDrawingForTick(void) {}

// A reset ejects every disk (DoMacReset in PROGMAIN.c), so they go back in on
// the tick after, pristine.
static int reinsert;

GLOBALOSGLUPROC WaitForNextTick(void) {
    int r;
    if (reinsert) {
        for (int i = 0; i < NumDrives; ++i) {
            if (Drives[i]) forget_writes(i);
        }
        insert_all();
        reinsert = 0;
    }
    r = afg_mac_tick(screencomparebuff);
    if (2 == r || (nullpr != SavedBriefMsg && SavedFatalMsg)) {
        ForceMacOff = trueblnr;
        return;
    }
    if (1 == r) {
        WantMacReset = trueblnr;
        reinsert = 1;
    }
    stamp();
    clock_gettime(CLOCK_MONOTONIC, &tick_start);
    ++OnTrueTime;
}

// ---- memory, as OSGLUSDL.c ----

static void ReserveAllocAll(void) {
    ReserveAllocOneBlock(&ROM, kROM_Size, 5, falseblnr);
    ReserveAllocOneBlock(&screencomparebuff, vMacScreenNumBytes, 5, trueblnr);
    EmulationReserveAlloc();
}

static int AllocMyMemory(void) {
    uimr n;
    ReserveAllocOffset = 0;
    ReserveAllocBigBlock = nullpr;
    ReserveAllocAll();
    n = ReserveAllocOffset;
    ReserveAllocBigBlock = (ui3p)calloc(1, n);
    if (NULL == ReserveAllocBigBlock) return 0;
    ReserveAllocOffset = 0;
    ReserveAllocAll();
    return n == ReserveAllocOffset;
}

// ---- the API afterglow calls ----

// `rom` is kROM_Size bytes; `disks` the image paths, boot disk first, opened
// here for the life of the process. Returns 0, or -1 if a disk will not open
// or memory runs out. Call once.
int mvx_init(const unsigned char *rom, const char *const *disks, int n) {
    if (booted) return -1;
    if (n > NumDrives) n = NumDrives;
    for (int i = 0; i < n; ++i) {
        if (!open_disk(i, disks[i])) return -1;
    }
    if (!AllocMyMemory()) return -1;
    memcpy(ROM, rom, kROM_Size);
    ROM_loaded = trueblnr;
    CurMacDelta = 0;
    stamp();
    insert_all();
    booted = 1;
    return 0;
}

// Runs the Mac until afg_mac_tick says stop (0) or the emulator gives up
// with a message of its own, such as a ROM it will not run (-1).
int mvx_run(void) {
    if (!booted) return -1;
    clock_gettime(CLOCK_MONOTONIC, &tick_start);
    ProgramMain();
    if (nullpr != SavedBriefMsg) {
        fprintf(stderr, "mac-engine: %s\n", SavedBriefMsg);
        return -1;
    }
    return 0;
}

// Mini vMac's SpeedValue: 0 is 1x, each step doubles. Above 1x, quiet
// stretches are not slowed back down: the speed is for getting somewhere.
void mvx_speed(int speed) {
    SpeedValue = (ui3b)speed;
    WantNotAutoSlow = speed > 0;
}

// The pointer's position on the Mac's screen and the button, as an event the
// Mac reads on its next interrupt. Only from inside afg_mac_tick.
void mvx_mouse(int h, int v, int down) {
    MyMousePositionSet((ui4r)h, (ui4r)v);
    MyMouseButtonSet(down ? trueblnr : falseblnr);
}

// `mkc` is a Mini vMac key code (MKC_* in minivmac/MYOSGLUE.h).
void mvx_key(int mkc, int down) {
    Keyboard_UpdateKeyMap((ui3r)mkc, down ? trueblnr : falseblnr);
}

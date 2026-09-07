/* fbfire — Doom-fire drawn straight into the display buffer, for the HDMI
 * screensaver on whichever Pi5 holds the monitor.
 *
 * Replaces the slow cacafire+fbterm+ncurses path. cacafire rendered ASCII fire
 * into a terminal that fbterm then repainted onto the framebuffer — an indirect
 * path that burned ~0.5 core for ~10 fps. This writes pixels straight into the
 * mapped buffer, which is the fast path (raw writes measured at hundreds of
 * MB/s).
 *
 * Two output backends, tried in order: fbdev (/dev/fb0) and DRM/KMS
 * (/dev/dri/card0). On Talos v1.14.0 only the second exists — see the backend
 * section below for why.
 *
 * Algorithm: the classic Doom PSX fire. A low-res fire grid (fw x fh) is seeded
 * white-hot along the bottom row and propagates upward each frame with a random
 * horizontal drift + decay, mapped through a 37-entry heat palette.
 *
 * Two render styles (FIRE_STYLE):
 *   blocks  — block-scale the fire grid straight to the panel (smooth/chunky;
 *             chunkiness governed by FIRE_SCALE).
 *   ascii   — retro terminal look: divide the panel into FIRE_CELL-pixel
 *             character cells, sample heat per cell, map heat -> a glyph from an
 *             ASCII ramp, and blit a built-in 8x8 font glyph (nearest-scaled to
 *             the cell), coloured by the heat palette. Recreates the cacafire/
 *             aalib vibe at direct-framebuffer speed.
 *
 * Geometry and pixel format are discovered at runtime — by ioctl on fbdev, or
 * from the connector's preferred mode on DRM — so it adapts to whatever the
 * screen is. Only 16bpp (RGB565) and 32bpp (XRGB8888) are implemented; that
 * covers the old Pi5 simplefb (16bpp) and DRM dumb buffers (32bpp).
 *
 * Env knobs:
 *   FB_DEVICE   fbdev device (default /dev/fb0)
 *   DRM_DEVICE  DRM device (default /dev/dri/card0)
 *   FIRE_FPS    target frames/sec (default 30)
 *   FIRE_STYLE  "ascii" (default) or "blocks"
 *   FIRE_SCALE  blocks mode: low-res fire grid = panel width / FIRE_SCALE (def 4)
 *   FIRE_CELL   ascii mode: character cell size in pixels (default 16, 8..64)
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <signal.h>
#include <sys/mman.h>
#include <sys/ioctl.h>
#include <linux/fb.h>
#include <xf86drm.h>
#include <xf86drmMode.h>

/* 37-step Doom fire palette (RGB). Index 0 = black (cooled), 36 = white-hot. */
static const uint8_t PAL[37][3] = {
  {0x07,0x07,0x07},{0x1F,0x07,0x07},{0x2F,0x0F,0x07},{0x47,0x0F,0x07},
  {0x57,0x17,0x07},{0x67,0x1F,0x07},{0x77,0x1F,0x07},{0x8F,0x27,0x07},
  {0x9F,0x2F,0x07},{0xAF,0x3F,0x07},{0xBF,0x47,0x07},{0xC7,0x47,0x07},
  {0xDF,0x4F,0x07},{0xDF,0x57,0x07},{0xDF,0x57,0x07},{0xD7,0x5F,0x07},
  {0xD7,0x5F,0x07},{0xD7,0x67,0x0F},{0xCF,0x6F,0x0F},{0xCF,0x77,0x0F},
  {0xCF,0x7F,0x0F},{0xCF,0x87,0x17},{0xC7,0x87,0x17},{0xC7,0x8F,0x17},
  {0xC7,0x97,0x1F},{0xBF,0x9F,0x1F},{0xBF,0x9F,0x1F},{0xBF,0xA7,0x27},
  {0xBF,0xA7,0x27},{0xBF,0xAF,0x2F},{0xB7,0xAF,0x2F},{0xB7,0xB7,0x2F},
  {0xB7,0xB7,0x37},{0xCF,0xCF,0x6F},{0xDF,0xDF,0x9F},{0xEF,0xEF,0xC7},
  {0xFF,0xFF,0xFF},
};

/* ASCII heat ramp: cool/empty -> hot/dense. 10 glyphs over heat 0..36. */
static const char RAMP[] = " .:-=+*#%@";
#define RAMP_LEN 10

/* Minimal 8x8 bitmap font, MSB = leftmost pixel. Only the glyphs used by RAMP
 * are defined; everything else renders blank (space). Public-domain style
 * 8x8 cell shapes, hand-tuned for a fiery, legible look. */
typedef struct { char c; uint8_t rows[8]; } Glyph;
static const Glyph FONT[] = {
  {' ', {0,0,0,0,0,0,0,0}},
  {'.', {0x00,0x00,0x00,0x00,0x00,0x18,0x18,0x00}},
  {':', {0x00,0x18,0x18,0x00,0x00,0x18,0x18,0x00}},
  {'-', {0x00,0x00,0x00,0x7E,0x00,0x00,0x00,0x00}},
  {'=', {0x00,0x00,0x7E,0x00,0x7E,0x00,0x00,0x00}},
  {'+', {0x00,0x18,0x18,0x7E,0x18,0x18,0x00,0x00}},
  {'*', {0x00,0x66,0x3C,0xFF,0x3C,0x66,0x00,0x00}},
  {'#', {0x66,0xFF,0x66,0x66,0x66,0xFF,0x66,0x00}},
  {'%', {0xC6,0xCC,0x18,0x30,0x66,0xC6,0x00,0x00}},
  {'@', {0x3C,0x42,0x99,0xA5,0xA5,0x9E,0x40,0x3C}},
};
#define FONT_LEN ((int)(sizeof(FONT)/sizeof(FONT[0])))

static const uint8_t *glyph_rows(char c){
  for(int i=0;i<FONT_LEN;i++) if(FONT[i].c==c) return FONT[i].rows;
  return FONT[0].rows; /* blank */
}

static volatile sig_atomic_t stop = 0;
static void on_sig(int s){ (void)s; stop = 1; }

/* ---------------------------------------------------------------------------
 * Output backends.
 *
 * Talos v1.14.0 builds its kernel with `# CONFIG_FB is not set`, so there is no
 * /dev/fbN on any node — verified on both pine and fir. CONFIG_DRM_FBDEV_EMULATION
 * is on, but that only feeds the in-kernel console (fbcon), which is why an
 * unconfigured HDMI port shows a Linux terminal rather than nothing. No device
 * tree overlay can change this: fbdev is compiled out, not merely unbound.
 *
 * So the real output path is DRM/KMS. A dumb buffer is exactly what this
 * renderer wants — a linear, CPU-mappable surface — and it lands as XRGB8888,
 * which the 32bpp branch below already handles. The fire code is untouched; only
 * how we obtain (pixels, stride, bpp) differs.
 *
 * The fbdev path is kept as a fallback so this still works on any host that does
 * have /dev/fb0, rather than silently requiring KMS.
 * ------------------------------------------------------------------------- */
typedef struct {
  int fd;
  uint32_t conn_id, crtc_id, fb_id, handle;
  drmModeCrtc *saved;          /* restored on exit so fbcon comes back */
  drmModeModeInfo mode;
  uint8_t *map; size_t size; uint32_t pitch;
} DrmOut;

static void drm_teardown(DrmOut *o){
  if(o->fd < 0) return;
  if(o->saved){
    drmModeSetCrtc(o->fd, o->saved->crtc_id, o->saved->buffer_id,
                   o->saved->x, o->saved->y, &o->conn_id, 1, &o->saved->mode);
    drmModeFreeCrtc(o->saved); o->saved = NULL;
  }
  if(o->map && o->map != MAP_FAILED) munmap(o->map, o->size);
  if(o->fb_id) drmModeRmFB(o->fd, o->fb_id);
  if(o->handle){ struct drm_mode_destroy_dumb d = { .handle = o->handle }; drmIoctl(o->fd, DRM_IOCTL_MODE_DESTROY_DUMB, &d); }
  close(o->fd); o->fd = -1;
}

/* Returns 0 on success, filling in width, height, stride, bpp and pixels. */
static int drm_setup(const char *path, DrmOut *o, uint32_t *W, uint32_t *H,
                     uint32_t *stride, uint32_t *bpp, uint8_t **pixels){
  memset(o, 0, sizeof(*o)); o->fd = -1;
  int fd = open(path, O_RDWR | O_CLOEXEC);
  if(fd < 0){ fprintf(stderr, "fbfire: open %s: %s\n", path, strerror(errno)); return -1; }
  o->fd = fd;
  /* Best-effort: fbcon holds the console, and we need to be master to modeset.
   * Not fatal if it fails — a container given the device is usually granted it. */
  drmSetMaster(fd);

  drmModeRes *res = drmModeGetResources(fd);
  if(!res){ fprintf(stderr, "fbfire: drmModeGetResources: %s\n", strerror(errno)); goto fail; }

  drmModeConnector *conn = NULL;
  for(int i = 0; i < res->count_connectors; i++){
    drmModeConnector *c = drmModeGetConnector(fd, res->connectors[i]);
    if(!c) continue;
    if(c->connection == DRM_MODE_CONNECTED && c->count_modes > 0){ conn = c; break; }
    drmModeFreeConnector(c);
  }
  if(!conn){ fprintf(stderr, "fbfire: no connected connector with modes\n"); drmModeFreeResources(res); goto fail; }
  o->conn_id = conn->connector_id;
  o->mode = conn->modes[0];   /* modes[0] is the preferred/native mode */

  /* Prefer the CRTC already driving this connector; otherwise take the first
   * one its encoders can use. */
  if(conn->encoder_id){
    drmModeEncoder *e = drmModeGetEncoder(fd, conn->encoder_id);
    if(e){ if(e->crtc_id) o->crtc_id = e->crtc_id; drmModeFreeEncoder(e); }
  }
  for(int i = 0; i < conn->count_encoders && !o->crtc_id; i++){
    drmModeEncoder *e = drmModeGetEncoder(fd, conn->encoders[i]);
    if(!e) continue;
    for(int j = 0; j < res->count_crtcs; j++)
      if(e->possible_crtcs & (1u << j)){ o->crtc_id = res->crtcs[j]; break; }
    drmModeFreeEncoder(e);
  }
  drmModeFreeConnector(conn);
  drmModeFreeResources(res);
  if(!o->crtc_id){ fprintf(stderr, "fbfire: no usable CRTC\n"); goto fail; }

  struct drm_mode_create_dumb creq = {
    .width = o->mode.hdisplay, .height = o->mode.vdisplay, .bpp = 32,
  };
  if(drmIoctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &creq)){
    fprintf(stderr, "fbfire: CREATE_DUMB: %s\n", strerror(errno)); goto fail;
  }
  o->handle = creq.handle; o->pitch = creq.pitch; o->size = creq.size;

  if(drmModeAddFB(fd, creq.width, creq.height, 24, 32, creq.pitch, creq.handle, &o->fb_id)){
    fprintf(stderr, "fbfire: drmModeAddFB: %s\n", strerror(errno)); goto fail;
  }

  struct drm_mode_map_dumb mreq = { .handle = creq.handle };
  if(drmIoctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &mreq)){
    fprintf(stderr, "fbfire: MAP_DUMB: %s\n", strerror(errno)); goto fail;
  }
  o->map = mmap(NULL, creq.size, PROT_READ|PROT_WRITE, MAP_SHARED, fd, mreq.offset);
  if(o->map == MAP_FAILED){ fprintf(stderr, "fbfire: mmap dumb: %s\n", strerror(errno)); goto fail; }
  memset(o->map, 0, creq.size);

  o->saved = drmModeGetCrtc(fd, o->crtc_id);
  if(drmModeSetCrtc(fd, o->crtc_id, o->fb_id, 0, 0, &o->conn_id, 1, &o->mode)){
    fprintf(stderr, "fbfire: drmModeSetCrtc: %s\n", strerror(errno)); goto fail;
  }

  *W = creq.width; *H = creq.height; *stride = creq.pitch; *bpp = 32; *pixels = o->map;
  fprintf(stderr, "fbfire: drm %s %ux%u@%uHz conn=%u crtc=%u pitch=%u\n",
          path, *W, *H, o->mode.vrefresh, o->conn_id, o->crtc_id, *stride);
  return 0;

fail:
  drm_teardown(o);
  return -1;
}

int main(void){
  const char *dev = getenv("FB_DEVICE"); if(!dev) dev = "/dev/fb0";
  int fps = 30; { const char *e = getenv("FIRE_FPS"); if(e){ int v=atoi(e); if(v>0&&v<=120) fps=v; } }
  int scale = 4; { const char *e = getenv("FIRE_SCALE"); if(e){ int v=atoi(e); if(v>=1&&v<=16) scale=v; } }
  int cell = 16; { const char *e = getenv("FIRE_CELL"); if(e){ int v=atoi(e); if(v>=8&&v<=64) cell=v; } }
  int ascii_mode = 1; { const char *e = getenv("FIRE_STYLE"); if(e && strcmp(e,"blocks")==0) ascii_mode = 0; }

  const char *drmdev = getenv("DRM_DEVICE"); if(!drmdev) drmdev = "/dev/dri/card0";

  uint32_t W = 0, H = 0, bpp = 0, stride = 0;
  uint8_t *fb = NULL; size_t maplen = 0;
  int fbfd = -1, using_drm = 0;
  DrmOut drm; memset(&drm, 0, sizeof(drm)); drm.fd = -1;

  /* fbdev first, since it needs no modeset and is what older hosts have. */
  fbfd = open(dev, O_RDWR);
  if(fbfd >= 0){
    struct fb_var_screeninfo vi; struct fb_fix_screeninfo fi;
    if(!ioctl(fbfd, FBIOGET_VSCREENINFO, &vi) && !ioctl(fbfd, FBIOGET_FSCREENINFO, &fi)){
      W = vi.xres; H = vi.yres; bpp = vi.bits_per_pixel; stride = fi.line_length;
      maplen = (size_t)stride * (vi.yres_virtual ? vi.yres_virtual : H);
      uint8_t *m = mmap(NULL, maplen, PROT_READ|PROT_WRITE, MAP_SHARED, fbfd, 0);
      if(m != MAP_FAILED){
        fb = m;
        fprintf(stderr, "fbfire: fbdev %s %ux%u %ubpp stride=%u\n", dev, W, H, bpp, stride);
      }
    }
    if(!fb){ close(fbfd); fbfd = -1; }
  }

  /* Then DRM/KMS, which is the only path on a Talos kernel built without
   * CONFIG_FB. Not a fallback in practice — it is the normal case here. */
  if(!fb && drm_setup(drmdev, &drm, &W, &H, &stride, &bpp, &fb) == 0){
    using_drm = 1; maplen = drm.size;
  }

  if(!fb){
    fprintf(stderr, "fbfire: no output — neither %s (fbdev) nor %s (drm) usable\n", dev, drmdev);
    return 1;
  }
  if(bpp != 16 && bpp != 32){
    fprintf(stderr, "fbfire: unsupported bpp=%u (only 16/32)\n", bpp);
    if(using_drm) drm_teardown(&drm);
    return 1;
  }
  fprintf(stderr, "fbfire: %s %ux%u %ubpp stride=%u fps=%d style=%s scale=%d cell=%d\n",
          using_drm ? "drm" : "fbdev", W, H, bpp, stride, fps,
          ascii_mode?"ascii":"blocks", scale, cell);

  signal(SIGTERM, on_sig); signal(SIGINT, on_sig);

  /* Fire-sim grid resolution.
   * blocks: panel/scale (block-scaled to panel).
   * ascii : one fire sample per character cell (W/cell x H/cell). */
  int fw, fh;
  if(ascii_mode){ fw = (int)(W/cell); fh = (int)(H/cell); }
  else          { fw = (int)(W/scale); fh = (int)(H/scale); }
  if(fw < 1) fw = 1;
  if(fh < 1) fh = 1;
  uint8_t *fire = calloc((size_t)fw * fh, 1);
  if(!fire){ fprintf(stderr,"fbfire: oom\n"); return 1; }
  for(int x=0; x<fw; x++) fire[(fh-1)*fw + x] = 36;  /* white-hot source row */

  /* Precompute palette for both pixel formats. */
  uint16_t pal16[37]; uint32_t pal32[37];
  for(int i=0;i<37;i++){
    uint8_t r=PAL[i][0], g=PAL[i][1], b=PAL[i][2];
    pal16[i] = (uint16_t)(((r>>3)<<11)|((g>>2)<<5)|(b>>3));
    pal32[i] = ((uint32_t)r<<16)|((uint32_t)g<<8)|b;
  }

  /* ascii mode: number of glyph cells and a clear of the panel once up-front
   * (cells only ever write their own area; the background stays black). */
  int cols = ascii_mode ? (int)(W/cell) : 0;
  int rows = ascii_mode ? (int)(H/cell) : 0;
  if(ascii_mode) memset(fb, 0, maplen);

  const long frame_ns = 1000000000L / fps;
  unsigned int rng = (unsigned int)time(NULL) ^ 0x9e3779b9u;

  while(!stop){
    struct timespec t0; clock_gettime(CLOCK_MONOTONIC, &t0);

    /* Propagate fire upward (Doom algorithm). */
    for(int x=0; x<fw; x++){
      for(int y=1; y<fh; y++){
        int src = y*fw + x;
        uint8_t v = fire[src];
        if(v == 0){ fire[src - fw] = 0; continue; }
        rng = rng*1103515245u + 12345u;
        int rnd = (rng >> 16) & 3;
        int dst = src - fw - (rnd & 1);
        if(dst < 0) dst = 0;
        int nv = v - (rnd & 1);
        fire[dst] = (uint8_t)(nv < 0 ? 0 : nv);
      }
    }

    if(!ascii_mode){
      /* ---- blocks: block-scale the fire grid into the framebuffer ---- */
      if(bpp == 16){
        for(uint32_t y=0; y<H; y++){
          int fy = (int)(y * fh / H);
          uint16_t *row = (uint16_t*)(fb + (size_t)y*stride);
          const uint8_t *frow = fire + (size_t)fy*fw;
          for(uint32_t x=0; x<W; x++) row[x] = pal16[frow[x*fw/W]];
        }
      } else {
        for(uint32_t y=0; y<H; y++){
          int fy = (int)(y * fh / H);
          uint32_t *row = (uint32_t*)(fb + (size_t)y*stride);
          const uint8_t *frow = fire + (size_t)fy*fw;
          for(uint32_t x=0; x<W; x++) row[x] = pal32[frow[x*fw/W]];
        }
      }
    } else {
      /* ---- ascii: blit a heat-mapped glyph per character cell ---- */
      for(int cy=0; cy<rows; cy++){
        for(int cx=0; cx<cols; cx++){
          uint8_t heat = fire[cy*fw + cx];           /* one sample per cell */
          int ri = heat * (RAMP_LEN-1) / 36;          /* heat -> ramp index */
          char ch = RAMP[ri];
          uint32_t ox = (uint32_t)cx*cell, oy = (uint32_t)cy*cell;
          if(ch == ' '){
            /* clear the cell (cooled) */
            for(int py=0; py<cell; py++){
              uint8_t *base = fb + (size_t)(oy+py)*stride;
              if(bpp==16){ uint16_t *r=(uint16_t*)base+ox; for(int px=0;px<cell;px++) r[px]=0; }
              else        { uint32_t *r=(uint32_t*)base+ox; for(int px=0;px<cell;px++) r[px]=0; }
            }
            continue;
          }
          const uint8_t *g = glyph_rows(ch);
          uint16_t c16 = pal16[heat]; uint32_t c32 = pal32[heat];
          for(int py=0; py<cell; py++){
            int gy = py * 8 / cell;                    /* nearest-scale 8 -> cell */
            uint8_t bits = g[gy];
            uint8_t *base = fb + (size_t)(oy+py)*stride;
            if(bpp==16){
              uint16_t *r = (uint16_t*)base + ox;
              for(int px=0; px<cell; px++){
                int gx = px * 8 / cell;
                r[px] = (bits & (0x80u >> gx)) ? c16 : 0;
              }
            } else {
              uint32_t *r = (uint32_t*)base + ox;
              for(int px=0; px<cell; px++){
                int gx = px * 8 / cell;
                r[px] = (bits & (0x80u >> gx)) ? c32 : 0;
              }
            }
          }
        }
      }
    }

    struct timespec t1; clock_gettime(CLOCK_MONOTONIC, &t1);
    long elapsed = (t1.tv_sec-t0.tv_sec)*1000000000L + (t1.tv_nsec-t0.tv_nsec);
    long sleep_ns = frame_ns - elapsed;
    if(sleep_ns > 0){ struct timespec ts={sleep_ns/1000000000L, sleep_ns%1000000000L}; nanosleep(&ts,NULL); }
  }

  memset(fb, 0, maplen);   /* clear on exit, don't leave a frozen frame */
  if(using_drm){
    drm_teardown(&drm);    /* also restores the previous CRTC so fbcon returns */
  } else {
    munmap(fb, maplen); close(fbfd);
  }
  free(fire);
  return 0;
}

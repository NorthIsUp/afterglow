/* fbfire — direct-to-framebuffer Doom-fire for the spruce/fir HDMI screensaver.
 *
 * Replaces the slow cacafire+fbterm+ncurses path. cacafire rendered ASCII fire
 * into a terminal that fbterm then repainted onto /dev/fb0 — an indirect path
 * that burned ~0.5 core for ~10 fps. This writes pixels straight into the
 * mmap'd framebuffer, which is the fast path (raw fb writes were measured at
 * hundreds of MB/s on fir).
 *
 * Algorithm: the classic Doom PSX fire. A low-res fire grid (FIRE_W x FIRE_H)
 * is seeded white-hot along the bottom row and propagates upward each frame
 * with a random horizontal drift + decay, mapped through a 37-entry heat
 * palette. The grid is block-scaled to the panel so per-frame work stays tiny
 * regardless of panel size (1920x1080 here).
 *
 * Framebuffer geometry + pixel format are queried at runtime via ioctl, so it
 * adapts to whatever the screen is (here: 1920x1080, RGB565, 16bpp). Currently
 * only 16bpp (RGB565) and 32bpp (XRGB8888) are implemented; that covers the
 * Pi5 simplefb (16bpp) and the common KMS case (32bpp).
 *
 * Env knobs:
 *   FB_DEVICE   framebuffer device (default /dev/fb0)
 *   FIRE_FPS    target frames/sec (default 30)
 *   FIRE_SCALE  low-res fire grid: panel width / FIRE_SCALE (default 4)
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

static volatile sig_atomic_t stop = 0;
static void on_sig(int s){ (void)s; stop = 1; }

int main(void){
  const char *dev = getenv("FB_DEVICE"); if(!dev) dev = "/dev/fb0";
  int fps = 30; { const char *e = getenv("FIRE_FPS"); if(e){ int v=atoi(e); if(v>0&&v<=120) fps=v; } }
  int scale = 4; { const char *e = getenv("FIRE_SCALE"); if(e){ int v=atoi(e); if(v>=1&&v<=16) scale=v; } }

  int fd = open(dev, O_RDWR);
  if(fd < 0){ fprintf(stderr, "fbfire: open %s: %s\n", dev, strerror(errno)); return 1; }

  struct fb_var_screeninfo vi; struct fb_fix_screeninfo fi;
  if(ioctl(fd, FBIOGET_VSCREENINFO, &vi) || ioctl(fd, FBIOGET_FSCREENINFO, &fi)){
    fprintf(stderr, "fbfire: ioctl screeninfo: %s\n", strerror(errno)); return 1;
  }
  uint32_t W = vi.xres, H = vi.yres, bpp = vi.bits_per_pixel;
  uint32_t stride = fi.line_length;
  size_t maplen = (size_t)stride * (vi.yres_virtual ? vi.yres_virtual : H);
  if(bpp != 16 && bpp != 32){
    fprintf(stderr, "fbfire: unsupported bpp=%u (only 16/32)\n", bpp); return 1;
  }
  fprintf(stderr, "fbfire: %s %ux%u %ubpp stride=%u fps=%d scale=%d\n",
          dev, W, H, bpp, stride, fps, scale);

  uint8_t *fb = mmap(NULL, maplen, PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0);
  if(fb == MAP_FAILED){ fprintf(stderr,"fbfire: mmap: %s\n", strerror(errno)); return 1; }

  signal(SIGTERM, on_sig); signal(SIGINT, on_sig);

  /* Low-res fire grid; block-scaled to the panel. */
  int fw = (int)(W / scale); if(fw < 1) fw = 1;
  int fh = (int)(H / scale); if(fh < 1) fh = 1;
  uint8_t *fire = calloc((size_t)fw * fh, 1);
  if(!fire){ fprintf(stderr,"fbfire: oom\n"); return 1; }
  /* Bottom row = white-hot source. */
  for(int x=0; x<fw; x++) fire[(fh-1)*fw + x] = 36;

  /* Precompute RGB565 / XRGB8888 palette for fast pixel writes. */
  uint16_t pal16[37]; uint32_t pal32[37];
  for(int i=0;i<37;i++){
    uint8_t r=PAL[i][0], g=PAL[i][1], b=PAL[i][2];
    pal16[i] = (uint16_t)(((r>>3)<<11)|((g>>2)<<5)|(b>>3));
    pal32[i] = ((uint32_t)r<<16)|((uint32_t)g<<8)|b;
  }

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
        int rnd = (rng >> 16) & 3;            /* 0..3 */
        int dst = src - fw - (rnd & 1);       /* drift left by 0/1 */
        if(dst < 0) dst = 0;
        int nv = v - (rnd & 1);               /* decay 0/1 */
        fire[dst] = (uint8_t)(nv < 0 ? 0 : nv);
      }
    }

    /* Block-scale the fire grid into the framebuffer. */
    if(bpp == 16){
      for(uint32_t y=0; y<H; y++){
        int fy = (int)(y * fh / H);
        uint16_t *row = (uint16_t*)(fb + (size_t)y*stride);
        const uint8_t *frow = fire + (size_t)fy*fw;
        for(uint32_t x=0; x<W; x++) row[x] = pal16[frow[x*fw/W]];
      }
    } else { /* 32bpp */
      for(uint32_t y=0; y<H; y++){
        int fy = (int)(y * fh / H);
        uint32_t *row = (uint32_t*)(fb + (size_t)y*stride);
        const uint8_t *frow = fire + (size_t)fy*fw;
        for(uint32_t x=0; x<W; x++) row[x] = pal32[frow[x*fw/W]];
      }
    }

    /* Frame pacing. */
    struct timespec t1; clock_gettime(CLOCK_MONOTONIC, &t1);
    long elapsed = (t1.tv_sec-t0.tv_sec)*1000000000L + (t1.tv_nsec-t0.tv_nsec);
    long sleep_ns = frame_ns - elapsed;
    if(sleep_ns > 0){ struct timespec ts={sleep_ns/1000000000L, sleep_ns%1000000000L}; nanosleep(&ts,NULL); }
  }

  /* Clear screen on exit so we don't leave a frozen frame. */
  memset(fb, 0, maplen);
  munmap(fb, maplen); free(fire); close(fd);
  return 0;
}

/* afterglow's own (MIT), in place of blip_buf 1.1.0's header (LGPL-2.1),
 * which mGBA bundles: the declarations mGBA calls, implemented as silence
 * by gba/blip_silent.c. */
#ifndef BLIP_BUF_H
#define BLIP_BUF_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct blip_t blip_t;

blip_t* blip_new(int sample_count);
void blip_set_rates(blip_t*, double clock_rate, double sample_rate);
void blip_clear(blip_t*);
void blip_add_delta(blip_t*, unsigned int clock_time, int delta);
void blip_add_delta_fast(blip_t*, unsigned int clock_time, int delta);
int blip_clocks_needed(const blip_t*, int sample_count);
void blip_end_frame(blip_t*, unsigned int clock_duration);
int blip_samples_avail(const blip_t*);
int blip_read_samples(blip_t*, short out[], int count, int stereo);
void blip_delete(blip_t*);

#ifdef __cplusplus
}
#endif

#endif

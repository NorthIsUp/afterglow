// blip_buf's API with nothing behind it, in place of the LGPL original mGBA
// bundles: the saver plays no sound, so no sample is ever wanted. Afterglow's
// own, MIT. mGBA only adds deltas while `blip_samples_avail` is under its
// buffer size, which 0 always is.

#include <stdlib.h>

#include <mgba/core/blip_buf.h>

struct blip_t {
	int unused;
};

blip_t* blip_new(int sample_count) {
	(void) sample_count;
	return calloc(1, sizeof(blip_t));
}

void blip_set_rates(blip_t* b, double clock_rate, double sample_rate) {
	(void) b;
	(void) clock_rate;
	(void) sample_rate;
}

void blip_clear(blip_t* b) {
	(void) b;
}

void blip_add_delta(blip_t* b, unsigned int clock_time, int delta) {
	(void) b;
	(void) clock_time;
	(void) delta;
}

void blip_add_delta_fast(blip_t* b, unsigned int clock_time, int delta) {
	(void) b;
	(void) clock_time;
	(void) delta;
}

int blip_clocks_needed(const blip_t* b, int sample_count) {
	(void) b;
	(void) sample_count;
	return 0;
}

void blip_end_frame(blip_t* b, unsigned int clock_duration) {
	(void) b;
	(void) clock_duration;
}

int blip_samples_avail(const blip_t* b) {
	(void) b;
	return 0;
}

int blip_read_samples(blip_t* b, short out[], int count, int stereo) {
	(void) b;
	(void) out;
	(void) count;
	(void) stereo;
	return 0;
}

void blip_delete(blip_t* b) {
	free(b);
}

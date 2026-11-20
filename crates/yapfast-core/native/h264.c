/* H264 encoding for Discord Go Live and the camera, through x264
 * (vendor/x264-src). The same interface as vp8.c, so Rust drives both alike,
 * with an effort besides. Frames come out in Annex B form, each NAL unit
 * after a start code. */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "x264.h"

struct yapfast_h264 {
  x264_t *encoder;
  /* What it was opened with, changed for each new bitrate. */
  x264_param_t param;
};

/* The rate control's buffer at `fps` frames a second, in milliseconds:
 * `ms`, and at least three frames. Over any span the encoder sends at most
 * its rate for that long and the buffer, so a keyframe or a scene cut goes
 * past the rate by that much at most, which the stream connection's limiter
 * (yapfast_discord::pacer) sends within it: a fifth of a second for a
 * stream held under a ceiling past Discord's documented 9 Mbit/s, half a
 * second otherwise and for the camera, whose keyframes keep more detail.
 * hwenc.c sizes the GPU encoders' alike. */
static unsigned rc_buffer_ms(unsigned fps, unsigned ms) {
  unsigned frames = fps ? 3000 / fps : 3000;
  return frames > ms ? frames : ms;
}

/* Aims for `kbps`, never above it over the buffer of rc_buffer_ms. */
static void set_rate(x264_param_t *param, unsigned kbps, unsigned ms) {
  unsigned fps = param->i_fps_den ? param->i_fps_num / param->i_fps_den : 0;
  unsigned long long buffer = (unsigned long long)kbps * rc_buffer_ms(fps, ms) / 1000;
  param->rc.i_bitrate = (int)kbps;
  param->rc.i_vbv_max_bitrate = (int)kbps;
  param->rc.i_vbv_buffer_size = buffer > 1 ? (int)buffer : 1;
}

/* A realtime encoder, as measured on screen recordings and games (see
 * docs/discord/go-live.md): x264's superfast preset, or faster at `effort`
 * 1, with zero latency: a frame out for each frame in, no B-frames or
 * lookahead, and a slice for each thread. Main profile, which Discord's own client
 * sends; adaptive quantization off, which kept text sharper and games no
 * worse; parameter sets before each IDR. The bitrate is capped over a buffer
 * of `buffer_ms` (set_rate), in average bitrate mode with the VBV's peak at the
 * target: no filler, so a calm screen sends less, and no HRD in the stream,
 * which a VBV changed while encoding would make wrong. Keyframes stay IDRs,
 * whole: Discord's viewers start decoding only at one, and ask for one when
 * they join, so periodic intra refresh would spare none of them. */
struct yapfast_h264 *yapfast_h264_new(unsigned width, unsigned height, unsigned fps,
                                      unsigned kbps, unsigned threads, unsigned effort,
                                      unsigned buffer_ms) {
  struct yapfast_h264 *h264 = calloc(1, sizeof *h264);
  if (!h264) return NULL;
  x264_param_t *param = &h264->param;
  if (x264_param_default_preset(param, effort ? "faster" : "superfast", "zerolatency") < 0) {
    free(h264);
    return NULL;
  }
  param->i_log_level = X264_LOG_NONE;
  param->i_width = (int)width;
  param->i_height = (int)height;
  param->i_csp = X264_CSP_I420;
  param->i_fps_num = fps;
  param->i_fps_den = 1;
  /* Timed by the frame rate: timing frames by their own times would hold
   * each one back until the next came, to learn how long it shows. */
  param->b_vfr_input = 0;
  param->i_threads = (int)threads;
  /* A new viewer waits at most this long for a picture. */
  param->i_keyint_max = (int)fps * 3;
  param->b_repeat_headers = 1;
  param->b_annexb = 1;
  param->rc.i_rc_method = X264_RC_ABR;
  set_rate(param, kbps, buffer_ms);
  param->rc.i_aq_mode = X264_AQ_NONE;
  /* libyuv converts captured pixels with BT.601's limited range. */
  param->vui.i_colorprim = 6;
  param->vui.i_transfer = 6;
  param->vui.i_colmatrix = 6;
  param->vui.b_fullrange = 0;
  if (x264_param_apply_profile(param, "main") < 0) {
    free(h264);
    return NULL;
  }
  h264->encoder = x264_encoder_open(param);
  if (!h264->encoder) {
    free(h264);
    return NULL;
  }
  return h264;
}

/* Encodes one I420 frame into `out`, with `pts` in 90 kHz units. Returns the
 * encoded size, 0 when the encoder gave no frame, or a negative number on
 * failure. `duration` is unused: x264 times frames by the frame rate. */
long yapfast_h264_encode(struct yapfast_h264 *h264, const unsigned char *y,
                         int y_stride, const unsigned char *u, int u_stride,
                         const unsigned char *v, int v_stride, long long pts,
                         unsigned long duration, int keyframe, unsigned char *out,
                         size_t capacity, int *is_key) {
  (void)duration;
  x264_picture_t picture, encoded;
  x264_picture_init(&picture);
  picture.img.i_csp = X264_CSP_I420;
  picture.img.i_plane = 3;
  picture.img.i_stride[0] = y_stride;
  picture.img.i_stride[1] = u_stride;
  picture.img.i_stride[2] = v_stride;
  picture.img.plane[0] = (uint8_t *)y;
  picture.img.plane[1] = (uint8_t *)u;
  picture.img.plane[2] = (uint8_t *)v;
  picture.i_pts = pts;
  picture.i_type = keyframe ? X264_TYPE_IDR : X264_TYPE_AUTO;
  x264_nal_t *nals = NULL;
  int count = 0;
  int size = x264_encoder_encode(h264->encoder, &nals, &count, &picture, &encoded);
  if (size < 0) return -1;
  if (size == 0 || count == 0) return 0;
  if ((size_t)size > capacity) return -2;
  /* x264 lays a frame's NAL units out one after another. */
  memcpy(out, nals[0].p_payload, (size_t)size);
  *is_key = encoded.b_keyframe;
  return size;
}

/* Changes the bitrate the encoder aims for, as the network allows, and its
 * buffer, as the stream's ceiling asks. Returns 0, or nonzero on failure. */
int yapfast_h264_set_bitrate(struct yapfast_h264 *h264, unsigned kbps, unsigned buffer_ms) {
  set_rate(&h264->param, kbps, buffer_ms);
  return x264_encoder_reconfig(h264->encoder, &h264->param) < 0;
}

void yapfast_h264_free(struct yapfast_h264 *h264) {
  x264_encoder_close(h264->encoder);
  free(h264);
}

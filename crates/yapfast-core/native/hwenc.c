/* Hardware encoding for Go Live, through FFmpeg's encoders in
 * vendor/ffmpeg-src: NVIDIA's NVENC, AMD's AMF on Windows and VA-API (AMD
 * and Intel) on Linux.
 * The same interface as h264.c, so Rust drives both alike: I420 frames in,
 * Annex B access units out, each NAL unit after a start code. Opening takes
 * the graphics adapter Rust chose (NVENC on a CUDA device by its index, AMF,
 * or VA-API on one render node), or tries each encoder in turn, and says
 * which one opened, or why none did.
 *
 * Settings, as measured on screen recordings and games
 * (docs/discord/go-live.md):
 * low-latency tuning, no B-frames, a frame out for each frame in, the
 * bitrate capped over half a second without padding a calm screen, H264's
 * Main profile, and parameter sets before each IDR. */
/* POSIX 2008 with XSI, for realpath. */
#define _XOPEN_SOURCE 700
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef __linux__
#include <unistd.h>
#endif

#include "libavcodec/avcodec.h"
#include "libavutil/hwcontext.h"
#include "libavutil/opt.h"
#include "libavutil/time.h"

/* The codecs, as Rust numbers them. */
enum { CODEC_H264, CODEC_HEVC, CODEC_AV1 };

/* Where to open the encoder, as Rust numbers it: the first that opens of
 * every family, NVENC on one CUDA device, AMF on its default adapter, or
 * VA-API on one render node. The last three index ENCODERS' columns, plus
 * one. */
enum { ON_ANY, ON_NVENC, ON_AMF, ON_VAAPI };

struct yapfast_hwenc {
  AVCodecContext *codec;
  /* VA-API's device, and the frame its encoder reads, on the GPU. */
  AVBufferRef *device;
  AVFrame *surface;
  /* The frame handed over, in memory: I420 for NVENC, NV12 for VA-API. */
  AVFrame *frame;
  AVPacket *packet;
  int64_t index;
  /* AMF hands a frame back a little after taking it, and is polled for it. */
  int poll;
};

/* FFmpeg's first error on this thread since it was cleared, for saying why
 * an encoder didn't open: the cause, where later ones only say what failed
 * because of it. */
static _Thread_local char first_error[160];

static void keep_error(void *context, int level, const char *format, va_list args) {
  (void)context;
  if (level > AV_LOG_ERROR || first_error[0]) return;
  vsnprintf(first_error, sizeof first_error, format, args);
  size_t length = strlen(first_error);
  if (length && first_error[length - 1] == '\n') first_error[length - 1] = 0;
}

static void set_rate(AVCodecContext *codec, unsigned kbps) {
  codec->bit_rate = (int64_t)kbps * 1000;
  codec->rc_max_rate = (int64_t)kbps * 1000;
  codec->rc_buffer_size = (int)(kbps * 500);
}

static void close_encoder(struct yapfast_hwenc *h) {
  avcodec_free_context(&h->codec);
  av_frame_free(&h->surface);
  av_frame_free(&h->frame);
  av_packet_free(&h->packet);
  av_buffer_unref(&h->device);
}

/* A render node for VA-API, and the libva driver for it. */
struct va_device {
  const char *path, *driver;
};

/* VA-API's device and a pool of NV12 surfaces for the encoder to read. */
static int open_vaapi(struct yapfast_hwenc *h, AVCodecContext *codec, struct va_device device) {
  AVDictionary *options = NULL;
  av_dict_set(&options, "driver", device.driver, 0);
  int ret = av_hwdevice_ctx_create(&h->device, AV_HWDEVICE_TYPE_VAAPI, device.path, options, 0);
  av_dict_free(&options);
  if (ret < 0) return ret;
  AVBufferRef *frames = av_hwframe_ctx_alloc(h->device);
  if (!frames) return AVERROR(ENOMEM);
  AVHWFramesContext *pool = (AVHWFramesContext *)frames->data;
  pool->format = AV_PIX_FMT_VAAPI;
  pool->sw_format = AV_PIX_FMT_NV12;
  pool->width = codec->width;
  pool->height = codec->height;
  pool->initial_pool_size = 2;
  ret = av_hwframe_ctx_init(frames);
  if (ret >= 0) codec->hw_frames_ctx = av_buffer_ref(frames);
  av_buffer_unref(&frames);
  if (ret < 0) return ret;
  h->surface = av_frame_alloc();
  return h->surface ? 0 : AVERROR(ENOMEM);
}

/* Opens `found` on `device` for VA-API, or for NVENC on CUDA device `gpu`
 * where that's 0 or more, the first that opens otherwise. */
static int open_encoder(struct yapfast_hwenc *h, const AVCodec *found, struct va_device device,
                        int gpu, unsigned codec_id, unsigned width, unsigned height,
                        unsigned fps, unsigned kbps) {
  int vaapi = device.path != NULL, amf = strstr(found->name, "_amf") != NULL;
  AVCodecContext *codec = h->codec = avcodec_alloc_context3(found);
  if (!codec) return AVERROR(ENOMEM);
  codec->width = (int)width;
  codec->height = (int)height;
  codec->time_base = (AVRational){1, (int)fps};
  codec->framerate = (AVRational){(int)fps, 1};
  /* A new viewer waits at most this long for a picture. */
  codec->gop_size = (int)fps * 3;
  codec->max_b_frames = 0;
  codec->pix_fmt = vaapi ? AV_PIX_FMT_VAAPI : amf ? AV_PIX_FMT_NV12 : AV_PIX_FMT_YUV420P;
  /* libyuv converts captured pixels with BT.601's limited range. */
  codec->color_primaries = AVCOL_PRI_SMPTE170M;
  codec->color_trc = AVCOL_TRC_SMPTE170M;
  codec->colorspace = AVCOL_SPC_SMPTE170M;
  codec->color_range = AVCOL_RANGE_MPEG;
  set_rate(codec, kbps);
  if (codec_id == CODEC_H264) codec->profile = AV_PROFILE_H264_MAIN;
  void *options = codec->priv_data;
  int ret = 0;
  if (vaapi) {
    av_opt_set(options, "rc_mode", "VBR", 0);
    av_opt_set_int(options, "async_depth", 1, 0);
    ret = open_vaapi(h, codec, device);
    if (ret < 0) return ret;
  } else if (amf) {
    /* Or FFmpeg holds each frame's packet back until the next frame. */
    codec->flags |= AV_CODEC_FLAG_LOW_DELAY;
    av_opt_set(options, "usage", "lowlatency", 0);
    av_opt_set(options, "quality", "balanced", 0);
    av_opt_set(options, "rc", "vbr_peak", 0);
    av_opt_set_int(options, "enforce_hrd", 1, 0);
    av_opt_set_int(options, "filler_data", 0, 0);
    av_opt_set_int(options, "async_depth", 1, 0);
    av_opt_set_int(options, "forced_idr", 1, 0);
    av_opt_set_int(options, "aud", 0, 0);
    if (codec_id == CODEC_AV1) {
      av_opt_set(options, "latency", "real_time", 0);
      av_opt_set(options, "header_insertion_mode", "frame", 0);
      av_opt_set_int(options, "bf", 0, 0);
    } else {
      av_opt_set_int(options, "latency", 1, 0);
    }
    if (codec_id == CODEC_HEVC) av_opt_set(options, "header_insertion_mode", "idr", 0);
    if (codec_id == CODEC_H264) {
      av_opt_set(options, "profile", "main", 0);
      av_opt_set_int(options, "bf", 0, 0);
      av_opt_set_int(options, "header_spacing", codec->gop_size, 0);
    }
    h->poll = 1;
  } else {
    av_opt_set(options, "preset", "p4", 0);
    av_opt_set(options, "tune", "ll", 0);
    av_opt_set(options, "rc", "vbr", 0);
    av_opt_set_int(options, "zerolatency", 1, 0);
    av_opt_set_int(options, "delay", 0, 0);
    av_opt_set_int(options, "forced-idr", 1, 0);
    if (codec_id == CODEC_H264) av_opt_set(options, "profile", "main", 0);
    /* CUDA numbers the cards in PCI bus order: the worker starts with
     * CUDA_DEVICE_ORDER=PCI_BUS_ID. */
    if (gpu >= 0) av_opt_set_int(options, "gpu", gpu, 0);
  }
  ret = avcodec_open2(codec, found, NULL);
  if (ret < 0) return ret;
  h->frame = av_frame_alloc();
  h->packet = av_packet_alloc();
  if (!h->frame || !h->packet) return AVERROR(ENOMEM);
  h->frame->format = vaapi || amf ? AV_PIX_FMT_NV12 : AV_PIX_FMT_YUV420P;
  h->frame->width = (int)width;
  h->frame->height = (int)height;
  return av_frame_get_buffer(h->frame, 0);
}

/* The encoders of each codec, by family (ON_NVENC, ON_AMF, ON_VAAPI), best
 * first; each system builds in those it has. */
static const char *const ENCODERS[][3] = {
    [CODEC_H264] = {"h264_nvenc", "h264_amf", "h264_vaapi"},
    [CODEC_HEVC] = {"hevc_nvenc", "hevc_amf", "hevc_vaapi"},
    [CODEC_AV1] = {"av1_nvenc", "av1_amf", "av1_vaapi"},
};

/* The libva drivers for each kernel driver whose GPUs encode, best first.
 * Naming the driver overrides LIBVA_DRIVER_NAME, which NixOS and others
 * set to nvidia for browsers, and which would fail every other GPU. NVIDIA's
 * own VA-API driver only decodes; NVENC encodes there. */
static const char *const VA_DRIVERS[][4] = {
    {"amdgpu", "radeonsi", NULL},
    {"radeon", "r600", NULL},
    {"i915", "iHD", "i965"},
    {"xe", "iHD", NULL},
    {"virtio_gpu", "virtio_gpu", NULL},
};

/* The name of the render node `node` resolves to, such as renderD128, into
 * `name`: /dev/dri/by-path's names are links to it. */
static void node_name(const char *node, char *name, size_t size) {
  const char *base = node;
#ifdef __linux__
  char *real = realpath(node, NULL);
  if (real) base = real;
#endif
  const char *slash = strrchr(base, '/');
  snprintf(name, size, "%s", slash ? slash + 1 : base);
#ifdef __linux__
  free(real);
#endif
}

/* The kernel driver of the render node named `name`, such as renderD128,
 * or an empty string. */
static void kernel_driver(const char *name, char *kernel, size_t size) {
  kernel[0] = 0;
#ifdef __linux__
  char link[96], target[256];
  snprintf(link, sizeof link, "/sys/class/drm/%s/device/driver", name);
  ssize_t length = readlink(link, target, sizeof target - 1);
  if (length <= 0) return;
  target[length] = 0;
  const char *base = strrchr(target, '/');
  base = base ? base + 1 : target;
  if (strlen(base) < size) memcpy(kernel, base, strlen(base) + 1);
#else
  (void)name;
  (void)size;
#endif
}

struct opening {
  unsigned codec_id, width, height, fps, kbps;
  char *report;
  size_t report_size, used;
};

/* Adds to the report why the encoder `label` names didn't open. */
static void note(struct opening *o, const char *label, const char *why) {
  if (o->used < o->report_size)
    o->used += (size_t)snprintf(o->report + o->used, o->report_size - o->used, "%s%s: %s",
                                o->used ? "; " : "", label, why);
}

/* Opens `found` on `device`, or on CUDA device `gpu` where that's 0 or
 * more, or adds why it didn't open to the report. */
static struct yapfast_hwenc *try_open(struct opening *o, const AVCodec *found,
                                      struct va_device device, int gpu) {
  const char *name = found->name;
  char label[96];
  if (device.path) {
    char node[64];
    node_name(device.path, node, sizeof node);
    snprintf(label, sizeof label, "%s (%s, %s)", name, node, device.driver);
  } else if (gpu >= 0) {
    snprintf(label, sizeof label, "%s (GPU %d)", name, gpu);
  } else {
    snprintf(label, sizeof label, "%s", name);
  }
  struct yapfast_hwenc *h = av_mallocz(sizeof *h);
  if (!h) return NULL;
  first_error[0] = 0;
  int ret = open_encoder(h, found, device, gpu, o->codec_id, o->width, o->height, o->fps,
                         o->kbps);
  if (ret >= 0) {
    snprintf(o->report, o->report_size, "%s", label);
    return h;
  }
  if (!first_error[0]) av_strerror(ret, first_error, sizeof first_error);
  note(o, label, first_error);
  close_encoder(h);
  av_free(h);
  return NULL;
}

/* Opens VA-API's `found` on the render node `node` with each libva driver
 * its kernel driver calls for: `kernel`, or the one sysfs names when that's
 * empty. A node with no such driver, or none at all, tries nothing. */
static struct yapfast_hwenc *try_vaapi(struct opening *o, const AVCodec *found, const char *node,
                                       const char *kernel) {
  char name[64], named[32];
  node_name(node, name, sizeof name);
  if (!kernel[0]) {
    kernel_driver(name, named, sizeof named);
    kernel = named;
  }
  for (size_t i = 0; i < sizeof VA_DRIVERS / sizeof *VA_DRIVERS; i++) {
    if (strcmp(kernel, VA_DRIVERS[i][0])) continue;
    for (const char *const *driver = VA_DRIVERS[i] + 1; *driver; driver++) {
      struct yapfast_hwenc *h = try_open(o, found, (struct va_device){node, *driver}, -1);
      if (h) return h;
    }
  }
  return NULL;
}

/* Opens an encoder of `codec_id` where `on` says (ON_ANY tries NVENC, AMF,
 * then VA-API on each render node with a driver for it; ON_NVENC opens on
 * CUDA device `gpu`, ON_VAAPI on the render node `node`, whose kernel
 * driver is `kernel` or, when empty, the one sysfs names), and writes its
 * name to `report`, or each one's failure when none opens. */
struct yapfast_hwenc *yapfast_hwenc_open(unsigned codec_id, unsigned on, const char *node,
                                         const char *kernel, int gpu, unsigned width,
                                         unsigned height, unsigned fps, unsigned kbps,
                                         char *report, size_t report_size) {
  if (report_size) report[0] = 0;
  if (codec_id > CODEC_AV1 || on > ON_VAAPI) return NULL;
  av_log_set_level(AV_LOG_ERROR);
  av_log_set_callback(keep_error);
  struct opening o = {codec_id, width, height, fps, kbps, report, report_size, 0};
  for (unsigned family = ON_NVENC; family <= ON_VAAPI; family++) {
    if (on != ON_ANY && on != family) continue;
    const char *name = ENCODERS[codec_id][family - ON_NVENC];
    const AVCodec *found = avcodec_find_encoder_by_name(name);
    if (!found) {
      if (on != ON_ANY) note(&o, name, "not in this build");
      continue;
    }
    struct yapfast_hwenc *h = NULL;
    if (family == ON_NVENC) {
      h = try_open(&o, found, (struct va_device){NULL, NULL}, on == ON_NVENC ? gpu : -1);
    } else if (family == ON_AMF) {
      h = try_open(&o, found, (struct va_device){NULL, NULL}, -1);
    } else if (on == ON_VAAPI) {
      size_t before = o.used;
      h = try_vaapi(&o, found, node ? node : "", kernel ? kernel : "");
      if (!h && o.used == before) note(&o, name, "no VA-API encoder for this graphics card");
    } else {
      for (int n = 0; n < 8 && !h; n++) {
        char path[32];
        snprintf(path, sizeof path, "/dev/dri/renderD%d", 128 + n);
        h = try_vaapi(&o, found, path, "");
      }
    }
    if (h) return h;
  }
  return NULL;
}

/* Copies an I420 picture into the frame the encoder reads: as it is for
 * NVENC, with its chroma planes interleaved for VA-API's and AMF's NV12. */
static int fill(struct yapfast_hwenc *h, const uint8_t *y, int y_stride, const uint8_t *u,
                int u_stride, const uint8_t *v, int v_stride) {
  AVFrame *frame = h->frame;
  int ret = av_frame_make_writable(frame);
  if (ret < 0) return ret;
  int width = frame->width, height = frame->height;
  for (int row = 0; row < height; row++)
    memcpy(frame->data[0] + row * frame->linesize[0], y + row * y_stride, (size_t)width);
  for (int row = 0; row < height / 2; row++) {
    const uint8_t *u_row = u + row * u_stride, *v_row = v + row * v_stride;
    if (frame->format == AV_PIX_FMT_NV12) {
      uint8_t *uv = frame->data[1] + row * frame->linesize[1];
      for (int x = 0; x < width / 2; x++) {
        uv[2 * x] = u_row[x];
        uv[2 * x + 1] = v_row[x];
      }
    } else {
      memcpy(frame->data[1] + row * frame->linesize[1], u_row, (size_t)width / 2);
      memcpy(frame->data[2] + row * frame->linesize[2], v_row, (size_t)width / 2);
    }
  }
  return 0;
}

/* Encodes one I420 frame into `out`. Returns the encoded size, 0 when the
 * encoder gave no frame, or a negative number on failure. `pts` and
 * `duration` are unused: frames are timed by the frame rate. */
long yapfast_hwenc_encode(struct yapfast_hwenc *h, const unsigned char *y, int y_stride,
                          const unsigned char *u, int u_stride, const unsigned char *v,
                          int v_stride, long long pts, unsigned long duration, int keyframe,
                          unsigned char *out, size_t capacity, int *is_key) {
  (void)pts;
  (void)duration;
  int ret = fill(h, y, y_stride, u, u_stride, v, v_stride);
  if (ret < 0) return -1;
  AVFrame *frame = h->frame;
  if (h->surface) {
    av_frame_unref(h->surface);
    ret = av_hwframe_get_buffer(h->codec->hw_frames_ctx, h->surface, 0);
    if (ret >= 0) ret = av_hwframe_transfer_data(h->surface, h->frame, 0);
    if (ret < 0) return -1;
    frame = h->surface;
  }
  frame->pts = h->index++;
  frame->pict_type = keyframe ? AV_PICTURE_TYPE_I : AV_PICTURE_TYPE_NONE;
  if (avcodec_send_frame(h->codec, frame) < 0) return -1;
  size_t size = 0;
  *is_key = 0;
  /* AMF waits a millisecond at most for each receive: polled for the
   * frame's packet for up to 100 ms, after which it comes with the next. */
  int64_t deadline = h->poll ? av_gettime_relative() + 100000 : 0;
  for (;;) {
    ret = avcodec_receive_packet(h->codec, h->packet);
    if (ret == AVERROR(EAGAIN) && !size && av_gettime_relative() < deadline) {
      /* Older drivers don't wait in AMF's receive at all. */
      av_usleep(250);
      continue;
    }
    if (ret < 0) break;
    if (size + (size_t)h->packet->size > capacity) {
      av_packet_unref(h->packet);
      return -2;
    }
    memcpy(out + size, h->packet->data, (size_t)h->packet->size);
    size += (size_t)h->packet->size;
    *is_key |= (h->packet->flags & AV_PKT_FLAG_KEY) != 0;
    av_packet_unref(h->packet);
  }
  if (ret != AVERROR(EAGAIN)) return -1;
  return (long)size;
}

/* Changes the bitrate aimed for, from the next frame on, without a
 * keyframe: patches/ in vendor/ffmpeg-src make NVENC and VA-API follow it.
 * Returns 0. */
int yapfast_hwenc_set_bitrate(struct yapfast_hwenc *h, unsigned kbps) {
  set_rate(h->codec, kbps);
  return 0;
}

void yapfast_hwenc_free(struct yapfast_hwenc *h) {
  close_encoder(h);
  av_free(h);
}

// PathForge's renderer (mobile/, libpathforge.a): the calls mobile/src/capi.rs makes.
// Strings it returns are the caller's to free with pfm_free.
#ifndef PATHFORGE_H
#define PATHFORGE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct PfmHandle PfmHandle;

void pfm_free(char *s);
/** The built-in presets' names, as a JSON array. */
char *pfm_preset_names(void);
/** A preset's scene as JSON, or NULL. */
char *pfm_preset_scene(const char *name);
/** Every scene setting as an editor builds itself from it, as JSON (see scene::form::form). */
char *pfm_form(void);
/** A scene file read as the desktop reads it, or NULL (see pfm_last_error). */
char *pfm_normalize(const char *json);
char *pfm_last_error(void);
/** Start the GPU renderer: the GPU's name, or "" where there is none. */
char *pfm_gpu_init(void);

/** Open a scene, drawn on the GPU when `gpu` and there is one; NULL on failure. */
PfmHandle *pfm_open(const char *json, const char *base_dir, bool gpu);
bool pfm_set_scene(PfmHandle *h, const char *json);
/* The phone's local time of day, hours (0..24), for scenes whose day and night follow the clock. */
void pfm_set_clock(float hour);
void pfm_close(PfmHandle *h);
/** [loop length (m), loop seconds, fps, canvas width, canvas height] into info[5]. */
bool pfm_info(PfmHandle *h, float *info);
/** True when the handle draws on the GPU now (a frame the GPU fails is drawn on the CPU). */
bool pfm_on_gpu(PfmHandle *h);
/** Draw into rgba (len bytes, at least w * h * 4): opaque RGBA rows. */
bool pfm_render(PfmHandle *h, float distance, float time, uint32_t w, uint32_t hh, uint8_t *rgba, size_t len);

/** The loop as an animated GIF or WebP ("gif" / "webp") in dir: its path, or NULL. */
char *pfm_export(const char *json, const char *dir, const char *name, const char *format, uint32_t w, uint32_t h, uint32_t fps, float quality, bool gpu);
/** Frames done, and of how many, of the export running now. */
void pfm_export_progress(uint32_t *done, uint32_t *total);
void pfm_export_cancel(void);

#endif

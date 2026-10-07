/* PathForge for games: render a PathForge scene every frame.
 *
 * Link libpath_forge_runtime (cdylib or staticlib from `cargo build --release -p path_forge_runtime`).
 * Distance walked and time are separate: walk at any speed, stop for a fight (flames and weather
 * keep moving), and the loop never shows a seam. Pixels are RGBA8, rows top to bottom.
 */
#ifndef PATH_FORGE_H
#define PATH_FORGE_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

typedef struct PfRuntime PfRuntime;

/* Open a scene file (its sprites, kits and grades are found relative to it). NULL on failure,
 * with the reason written to err (may be NULL). */
PfRuntime *pf_runtime_open(const char *path, char *err, size_t err_len);
/* A scene from JSON text; files it names are relative to base_dir (may be NULL). */
PfRuntime *pf_runtime_from_json(const char *json, const char *base_dir, char *err, size_t err_len);
void pf_runtime_free(PfRuntime *rt);

/* Metres before the view repeats, and seconds before every timed effect repeats. */
float pf_runtime_loop_length(const PfRuntime *rt);
float pf_runtime_loop_seconds(const PfRuntime *rt);
/* The scene's own canvas size. 0 on success. */
int pf_runtime_canvas(const PfRuntime *rt, uint32_t *width, uint32_t *height);

/* Render the view `distance` metres along the path at `time` seconds into out
 * (width * height * 4 bytes). 0 on success; -1 null handle or buffer; -2 buffer too small;
 * -3 rendering failed. */
int pf_runtime_render(PfRuntime *rt, float distance, float time, uint32_t width, uint32_t height, uint8_t *out, size_t out_len);

/* Where a point appears on a width x height screen: x metres right of the path centre, y up,
 * d ahead. Writes {x, y} pixels to out_xy. 1 in front of the camera, 0 behind, -1 null handle. */
int pf_runtime_project(const PfRuntime *rt, float distance, float x, float y, float d, uint32_t width, uint32_t height, float *out_xy);
/* Metres ahead of the ground seen on screen row `row`; negative above the horizon. */
float pf_runtime_ground_distance(const PfRuntime *rt, float distance, float row, uint32_t width, uint32_t height);

/* ── The runtime's own walk: transitions, forks and journeys ────────────────
 * Instead of passing distance and time to pf_runtime_render, let the runtime walk: pf_runtime_step
 * each frame, then pf_runtime_frame. Transitions and forks run inside the walk; when one ends,
 * the next scene becomes the current one. */

/* Open a journey file (scenes and where each leads). NULL on failure. */
PfRuntime *pf_runtime_open_journey(const char *path, char *err, size_t err_len);
/* Walk on by dt seconds at speed metres per second (0 stands still; time still passes). */
int pf_runtime_step(PfRuntime *rt, float dt, float speed);
/* Render the walk's current frame (as pf_runtime_render). */
int pf_runtime_frame(PfRuntime *rt, uint32_t width, uint32_t height, uint8_t *out, size_t out_len);
/* Walk from here into the scene file `path`. transition_json may be NULL, or override choices,
 * e.g. {"threshold":"CaveMouth","approach_m":30}. 0 on success, -2 with the reason in err. */
int pf_runtime_transition_to(PfRuntime *rt, const char *path, const char *transition_json, char *err, size_t err_len);
/* A fork ahead into the scene files left and right (fork_json may be NULL). Then choose. */
int pf_runtime_fork(PfRuntime *rt, const char *left, const char *right, const char *fork_json, char *err, size_t err_len);
/* Take a branch: 0 left, 1 right. 1 if taken, 0 if there is no fork or the default was already taken. */
int pf_runtime_choose(PfRuntime *rt, int side);
/* Walk to where the journey leads from the current stop. 0 on success, -2 with the reason in err. */
int pf_runtime_go(PfRuntime *rt, char *err, size_t err_len);
/* Where the walk stands, as JSON: {"scene","distance","time","stop","progress","choose_within"}.
 * choose_within is non-null while a fork waits for a choice (metres left). Returns length or -1. */
int pf_runtime_state(const PfRuntime *rt, char *out, size_t out_len);
/* Where a point d metres ahead appears now (as pf_runtime_project, for the walk). */
int pf_runtime_project_now(const PfRuntime *rt, float x, float y, float d, uint32_t width, uint32_t height, float *out_xy);

#ifdef __cplusplus
}
#endif
#endif

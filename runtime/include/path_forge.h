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
/* pf_runtime_go, and what it planned: JSON {"kind","notes","warnings"} written into out. Returns
 * the JSON's full length (call again with a larger buffer if it did not fit), -1 for a null
 * handle, or -2 with the reason in out. */
int pf_runtime_go_plan(PfRuntime *rt, char *out, size_t out_len);
/* Where the walk stands, as JSON: {"scene","distance","time","stop","progress","choose_within","day","intro_left","swing_left"}.
 * choose_within is non-null while a fork waits for a choice (metres left); intro_left is the
 * seconds left of the intro (the camera's flight in or swing in) while it plays, so the game can
 * hold the player's controls until it lands; swing_left the same for a pf_runtime_swing_to swing.
 * Returns length or -1. */
int pf_runtime_state(const PfRuntime *rt, char *out, size_t out_len);
/* Where a point d metres ahead of the walker appears now (as pf_runtime_project, for the walk;
 * while a swing has the camera standing behind the walker, d is still the walker's). */
int pf_runtime_project_now(const PfRuntime *rt, float x, float y, float d, uint32_t width, uint32_t height, float *out_xy);

/* ── Day and night ──────────────────────────────────────────────────────────
 * Every scene with an open sky can run through the day: the sun crosses the sky and sets, the
 * light, sky and fog follow it, stars and moon come out and lamps light at dusk. */

/* Set the hour (0..24: 6 sunrise, 12 noon, 18 sunset, 0 midnight) on every scene drawn, turning
 * its day and night on; a negative hour gives each scene back its own. */
int pf_runtime_set_hour(PfRuntime *rt, float hour);
/* Play the intro again from the top: the camera's flight in (camera.moves.fly_in: it comes down
 * from above) and swing in (camera.moves.swing_in: it comes round from another view into the
 * scene's own) over the next seconds of pf_runtime_step (it plays once at the start). */
int pf_runtime_restart_intro(PfRuntime *rt);
/* Swing the camera from where it is now to another view (interface 3): JSON, every field
 * optional, {"yaw","side","eye_height","horizon","zoom"} the view (missing ones stay as they
 * are) and {"seconds","hold","ease","overshoot","orbit","bars"} how (as camera.moves.swing_in;
 * 0 seconds is 2.5), e.g. {"yaw":-90,"side":6} round to a side-scroller's view or {"yaw":0,
 * "side":0} back down the path. It circles the walker, dissolving the near side of the world out
 * of its way, over the next seconds of pf_runtime_step; the view stays once it lands
 * (pf_runtime_state's swing_left counts down). 0 begun, -1 null handle, -2 with the reason in err. */
int pf_runtime_swing_to(PfRuntime *rt, const char *json, char *err, size_t err_len);
/* The camera's view now (interface 3), JSON {"yaw","side","back","eye_height","horizon","zoom"}:
 * where a swing or the intro has it (back: metres behind the walker). Returns length or -1. */
int pf_runtime_view(const PfRuntime *rt, char *out, size_t out_len);
/* Let the days pass: one day every `seconds` of pf_runtime_step time (0 holds the hour). */
int pf_runtime_set_day_length(PfRuntime *rt, float seconds);
/* The day now: writes the hour and the daylight (0 night .. 1 day; either may be NULL) and
 * returns 1; 0 when the scene has no open sky; -1 for a null handle. pf_runtime_state's "day"
 * also gives "altitude" (sun height, -1..1), "is_day" and "phase" ("night", "dawn", "day", "dusk"). */
int pf_runtime_day(const PfRuntime *rt, float *hour, float *daylight);

/* ── For programs that load this library at run time ────────────────────────
 * An editor's preview, say, which must build whether or not PathForge is installed. */

/* The version of this interface (2 added the three functions below and pf_runtime_go_plan; 3
 * pf_runtime_swing_to and pf_runtime_view). */
uint32_t pf_abi_version(void);
/* Open a place as a journey stop names one: a scene file (relative to base_dir, which may be
 * NULL) or a built-in preset as "preset:Name". NULL on failure, with the reason in err. */
PfRuntime *pf_runtime_open_place(const char *name, const char *base_dir, char *err, size_t err_len);
/* A journey file's outline: JSON {"start","stops":[{"id","scene","next"}],"problems":[...]},
 * "next" being "end", {"go": to} or {"fork": [left, right]}. Returns the full length, or -2
 * with the reason in out. */
int pf_journey_outline(const char *path, char *out, size_t out_len);

#ifdef __cplusplus
}
#endif
#endif

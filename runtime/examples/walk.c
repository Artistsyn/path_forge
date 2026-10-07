/* Walk a PathForge scene for a few seconds of game time and report the frame cost.
 *   cc runtime/examples/walk.c -Iruntime/include -Ltarget/release -lpath_forge_runtime -o walk
 *   ./walk scene.json 270 480
 */
#include "path_forge.h"
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec * 1e-9; }

int main(int argc, char **argv) {
    if (argc < 2) { fprintf(stderr, "usage: walk scene.json [width height]\n"); return 2; }
    uint32_t w = argc > 3 ? (uint32_t)atoi(argv[2]) : 270, h = argc > 3 ? (uint32_t)atoi(argv[3]) : 480;
    char err[512];
    PfRuntime *rt = pf_runtime_open(argv[1], err, sizeof err);
    if (!rt) { fprintf(stderr, "open failed: %s\n", err); return 1; }
    size_t len = (size_t)w * h * 4;
    uint8_t *px = malloc(len);
    float loop = pf_runtime_loop_length(rt), dist = 0, t = 0, speed = 3.5f, dt = 1.0f / 60.0f;
    pf_runtime_render(rt, 0, 0, w, h, px, len); /* warm caches */
    int frames = 120;
    double t0 = now();
    for (int i = 0; i < frames; i++) {
        /* stand still for the middle second, as if fighting */
        float s = (i >= 40 && i < 100) ? 0.0f : speed;
        dist += s * dt; if (dist >= loop) dist -= loop;
        t += dt;
        if (pf_runtime_render(rt, dist, t, w, h, px, len) != 0) { fprintf(stderr, "render failed\n"); return 1; }
    }
    double ms = (now() - t0) * 1000.0 / frames;
    float xy[2];
    pf_runtime_project(rt, dist, 0, 0, 8, w, h, xy);
    printf("%ux%u: %.1f ms/frame; an enemy 8 m ahead stands at (%.0f, %.0f); row %.0f is %.2f m ahead\n",
           w, h, ms, xy[0], xy[1], xy[1], pf_runtime_ground_distance(rt, dist, xy[1], w, h));
    free(px);
    pf_runtime_free(rt);
    return 0;
}

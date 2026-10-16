package com.pathforge.mobile;

import java.nio.ByteBuffer;

/** PathForge's renderer (mobile/, libpathforge.so). Scenes are JSON; a handle is one scene
 *  being drawn, used from one thread at a time. */
final class Native {
    static { System.loadLibrary("pathforge"); }

    private Native() {}

    /** The built-in presets' names, as a JSON array. */
    static native String presetNames();
    /** A preset's scene, or null. */
    static native String presetScene(String name);
    /** Every scene setting as an editor builds itself from it, as JSON (see scene::form::form). */
    static native String form();
    /** A scene file read as the desktop reads it (older formats converted), or null (see lastError). */
    static native String normalize(String json);
    static native String lastError();

    /** Start the GPU renderer: the adapter's name, or "" where there is none (see lastError). */
    static native String gpuInit();
    /** Open a scene, drawn on the GPU when `gpu` and there is one; 0 on failure (see lastError).
     *  `baseDir` is where its files are, or "". A frame the GPU fails is drawn on the CPU. */
    static native long open(String json, String baseDir, boolean gpu);
    /** "gpu" or "cpu": which engine draws a handle's frames now. */
    static native String engine(long handle);
    static native boolean setScene(long handle, String json);
    static native void close(long handle);
    /** [loop length (m), loop seconds, fps, canvas width, canvas height]. */
    static native float[] info(long handle);
    /** The loop as an animated GIF or WebP ("gif" / "webp") in `dir`, seamless: its path, or null
     *  (see lastError). Blocks until done; exportProgress tells how far, exportCancel stops it. */
    static native String exportAnimation(String json, String dir, String name, String format, int w, int h, int fps, float quality, boolean gpu);
    /** [frames done, of how many] of the export running now. */
    static native int[] exportProgress();
    static native void exportCancel();
    /** Draw into a direct buffer of w * h * 4 bytes (RGBA, as an ARGB_8888 bitmap holds it). */
    static native boolean render(long handle, float distance, float time, int w, int h, ByteBuffer out);
    /** A new variation of a scene ("Surprise me"): the parts named in `keep` (comma separated:
     *  camera, path, setting, sky, lighting, props, atmosphere, look) stay, the rest come from
     *  presets picked by `seed`. Null on failure (see lastError). */
    static native String remix(String json, String keep, int seed);
    /** A scene as a share code (one short line of text starting PF1.), or null. */
    static native String shareCode(String json);
    /** The scene a share code holds, or null (see lastError). */
    static native String unshare(String code);
    /** A QR code of `text`: its width in modules, then one byte a module (1 dark), row by row;
     *  null when it is too long. */
    static native byte[] qr(String text);

    /** The phone's local time of day, hours (0..24), for scenes whose day and night follow the clock. */
    static native void setClock(float hour);

    /** Hand the phone's time of day to scenes that follow the clock. */
    static void tickClock() {
        java.util.Calendar c = java.util.Calendar.getInstance();
        setClock(c.get(java.util.Calendar.HOUR_OF_DAY) + c.get(java.util.Calendar.MINUTE) / 60f + c.get(java.util.Calendar.SECOND) / 3600f);
    }
}

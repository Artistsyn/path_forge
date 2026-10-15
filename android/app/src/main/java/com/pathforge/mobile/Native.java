package com.pathforge.mobile;

import java.nio.ByteBuffer;

/** PathForge's renderer (android/native, libpathforge.so). Scenes are JSON; a handle is one scene
 *  being drawn, used from one thread at a time. */
final class Native {
    static { System.loadLibrary("pathforge"); }

    private Native() {}

    /** The built-in presets' names, as a JSON array. */
    static native String presetNames();
    /** A preset's scene, or null. */
    static native String presetScene(String name);
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
    /** Draw into a direct buffer of w * h * 4 bytes (RGBA, as an ARGB_8888 bitmap holds it). */
    static native boolean render(long handle, float distance, float time, int w, int h, ByteBuffer out);
}

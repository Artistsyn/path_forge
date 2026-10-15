package com.pathforge.mobile;

import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Matrix;
import android.graphics.Paint;
import android.os.SystemClock;
import android.view.SurfaceHolder;
import android.view.SurfaceView;

import java.nio.ByteBuffer;

/** The scene walking past, rendered live on its own thread. It renders a fraction of the screen's
 *  size, as small as it needs to keep up (about 20-30 frames a second), and lets the GPU scale it. */
final class PreviewView extends SurfaceView implements SurfaceHolder.Callback, Runnable {
    private final Object lock = new Object();
    private volatile String scene;
    private volatile boolean changed;
    private volatile boolean reopen;
    /** Told about twice a second what is drawing and how fast. */
    interface Stats { void at(String engine, int w, int h, float fps); }
    private volatile Stats stats;
    private volatile boolean running;
    private Thread thread;
    private boolean surfaceOk;
    private volatile int vw, vh;
    private final long t0 = SystemClock.uptimeMillis();
    /** Where the walk is now: metres along and seconds into the loop (for stills of this moment). */
    private volatile float nowDistance, nowTime;

    PreviewView(Context c) {
        super(c);
        getHolder().addCallback(this);
    }

    void setScene(String json) {
        scene = json;
        changed = true;
    }

    /** Open the scene again (after the GPU is switched on or off). */
    void reopen() {
        reopen = true;
        changed = true;
    }

    void setStats(Stats s) { stats = s; }

    /** While editing: the whole frame shown in the top `fraction` of the view, below `top` pixels
     *  (the status bar); 0 fills the view again. */
    void setFitTop(float fraction, int top) {
        fitTop = fraction;
        fitInset = top;
    }
    private volatile float fitTop;
    private volatile int fitInset;

    float distance() { return nowDistance; }
    float time() { return nowTime; }

    void resume() {
        if (running) return;
        running = true;
        thread = new Thread(this, "pathforge-preview");
        thread.start();
    }

    void pause() {
        running = false;
        Thread t = thread;
        thread = null;
        if (t != null) {
            try { t.join(2000); } catch (InterruptedException ignored) { }
        }
    }

    @Override public void surfaceCreated(SurfaceHolder holder) { }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        synchronized (lock) {
            vw = width;
            vh = height;
            surfaceOk = true;
        }
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        synchronized (lock) { surfaceOk = false; }
    }

    @Override
    public void run() {
        long h = 0;
        float len = 24f, secs = 10f;
        Paint paint = Frame.paint(false);
        Matrix m = new Matrix();
        // Render at a 1/div of the screen's width, adapting to how fast this phone draws the scene.
        int div = 3;
        float avg = 30f;
        int frames = 0;
        boolean first = true;
        long statsAt = SystemClock.uptimeMillis();
        ByteBuffer buf = null;
        Bitmap bmp = null;
        try {
            while (running) {
                long frameStart = SystemClock.uptimeMillis();
                if (changed) {
                    changed = false;
                    String s = scene;
                    if (reopen && h != 0) { Native.close(h); h = 0; first = true; }
                    reopen = false;
                    if (h == 0) h = Gpu.open(getContext(), s); else if (!Native.setScene(h, s)) { Native.close(h); h = Gpu.open(getContext(), s); }
                    if (h != 0) {
                        float[] info = Native.info(h);
                        len = info[0];
                        secs = Math.max(0.1f, info[1]);
                    }
                    paint = Frame.paint(Look.pixelated(s));
                }
                int w = vw, hh = vh;
                if (h == 0 || w <= 0 || hh <= 0) { sleep(50); continue; }
                int rw = Math.max(64, w / div), rh = Math.max(64, Math.round(hh * (rw / (float) w)));
                if (bmp == null || bmp.getWidth() != rw || bmp.getHeight() != rh) {
                    buf = ByteBuffer.allocateDirect(rw * rh * 4);
                    bmp = Bitmap.createBitmap(rw, rh, Bitmap.Config.ARGB_8888);
                }
                float t = ((SystemClock.uptimeMillis() - t0) / 1000f) % secs;
                float d = t / secs * len;
                long a = System.nanoTime();
                boolean ok = Native.render(h, d, t, rw, rh, buf);
                avg = avg * 0.8f + (System.nanoTime() - a) / 1e6f * 0.2f;
                if (ok) {
                    nowDistance = d;
                    nowTime = t;
                    buf.rewind();
                    bmp.copyPixelsFromBuffer(buf);
                    buf.rewind();
                    draw(bmp, paint, m);
                    if (first) { Gpu.rendered(getContext(), h); first = false; }
                    frames++;
                }
                long now = SystemClock.uptimeMillis();
                Stats st = stats;
                if (now - statsAt >= 500) {
                    if (st != null) st.at(Native.engine(h), rw, rh, frames * 1000f / (now - statsAt));
                    frames = 0;
                    statsAt = now;
                }
                // As much of the screen's size as keeps up: halve at best, a tenth at worst.
                if (avg > 45f && div < 10) { div++; avg = 30f; }
                else if (avg < 16f && div > 2) { div--; avg = 30f; }
                // At most about 30 frames a second.
                long spare = 33 - (SystemClock.uptimeMillis() - frameStart);
                if (spare > 0) sleep(spare);
            }
        } finally {
            if (h != 0) Native.close(h);
        }
    }

    private void draw(Bitmap b, Paint paint, Matrix m) {
        synchronized (lock) {
            if (!surfaceOk) return;
            SurfaceHolder holder = getHolder();
            Canvas c;
            try {
                c = holder.lockHardwareCanvas();
            } catch (IllegalStateException | IllegalArgumentException e) {
                c = holder.lockCanvas();
            }
            if (c == null) return;
            try {
                float f = fitTop;
                if (f > 0) {
                    c.drawColor(android.graphics.Color.BLACK);
                    Frame.fit(c, b, 0, fitInset, c.getWidth(), Math.round(c.getHeight() * f) - fitInset, paint, m);
                } else {
                    Frame.cover(c, b, c.getWidth(), c.getHeight(), paint, m);
                }
            } finally {
                holder.unlockCanvasAndPost(c);
            }
        }
    }

    private static void sleep(long ms) {
        try { Thread.sleep(ms); } catch (InterruptedException ignored) { }
    }
}

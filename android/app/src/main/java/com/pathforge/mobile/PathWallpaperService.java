package com.pathforge.mobile;

import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Matrix;
import android.graphics.Paint;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.SystemClock;
import android.service.wallpaper.WallpaperService;
import android.view.SurfaceHolder;

import java.io.File;
import java.io.IOException;

/** The live wallpaper: plays the frames the app baked (Bake), round and round, only while it can
 *  be seen. If there are none yet (the app's data was cleared), it bakes them itself first. */
public final class PathWallpaperService extends WallpaperService {
    @Override
    public Engine onCreateEngine() { return new PathEngine(); }

    private final class PathEngine extends Engine {
        private HandlerThread thread;
        private Handler handler;
        private Bake.Player player;
        private int version = -1;
        private volatile boolean visible;
        private volatile int sw, sh;
        private final long start = SystemClock.uptimeMillis();
        private volatile String status = "Preparing your path…";
        private volatile Thread baking;
        private volatile boolean destroyed;
        private Paint paint = Frame.paint(false);
        private final Matrix matrix = new Matrix();
        private final Paint text = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Runnable tick = this::tick;

        @Override
        public void onCreate(SurfaceHolder holder) {
            super.onCreate(holder);
            setTouchEventsEnabled(false);
            thread = new HandlerThread("pathforge-wallpaper");
            thread.start();
            handler = new Handler(thread.getLooper());
            text.setColor(Color.WHITE);
            text.setTextAlign(Paint.Align.CENTER);
        }

        @Override
        public void onDestroy() {
            super.onDestroy();
            destroyed = true;
            visible = false;
            Thread b = baking;
            if (b != null) b.interrupt();
            handler.removeCallbacksAndMessages(null);
            handler.post(() -> {
                release();
                thread.quitSafely();
            });
        }

        @Override
        public void onSurfaceChanged(SurfaceHolder holder, int format, int width, int height) {
            super.onSurfaceChanged(holder, format, width, height);
            sw = width;
            sh = height;
            handler.post(() -> { prepare(); tick(); });
        }

        @Override
        public void onSurfaceDestroyed(SurfaceHolder holder) {
            visible = false;
            handler.removeCallbacks(tick);
            // Wait for a frame being drawn, so the surface is not drawn to after it has gone.
            final Object done = new Object();
            synchronized (done) {
                handler.post(() -> { synchronized (done) { done.notifyAll(); } });
                try { done.wait(500); } catch (InterruptedException ignored) { }
            }
            super.onSurfaceDestroyed(holder);
        }

        @Override
        public void onVisibilityChanged(boolean v) {
            visible = v;
            handler.removeCallbacks(tick);
            if (v) handler.post(() -> { prepare(); tick(); });
        }

        /** Open the newest frames, or start baking them. On the engine's thread. */
        private void prepare() {
            int v = Bake.version(PathWallpaperService.this);
            if (player != null && v == version) return;
            File f = Bake.file(PathWallpaperService.this);
            if (f.exists() && baking == null) {
                try {
                    Bake.Player p = Bake.Player.open(f);
                    release();
                    player = p;
                    version = v;
                    paint = Frame.paint(p.pixel);
                    return;
                } catch (IOException ignored) {
                    // Bake it again below.
                }
            }
            if (player == null && baking == null && sw > 0 && sh > 0) bakeHere();
        }

        private void bakeHere() {
            final int w = sw, h = sh;
            Thread t = new Thread(() -> {
                try {
                    Look look = Look.load(PathWallpaperService.this);
                    Bake.bake(PathWallpaperService.this, look.scene(), look.quality, w, h, new Bake.Progress() {
                        @Override public void at(int done, int total) {
                            status = "Preparing your path… " + (100 * done / total) + "%";
                            if (visible) handler.post(tick);
                        }
                        @Override public boolean cancelled() { return destroyed; }
                    });
                    Bake.bump(PathWallpaperService.this);
                } catch (IOException | RuntimeException e) {
                    status = "Open PathForge to set this wallpaper up";
                } finally {
                    baking = null;
                    if (!destroyed) handler.post(() -> { prepare(); tick(); });
                }
            }, "pathforge-bake");
            t.setPriority(Thread.MIN_PRIORITY);
            baking = t;
            t.start();
        }

        private void release() {
            if (player != null) {
                try { player.close(); } catch (IOException ignored) { }
                player = null;
            }
        }

        /** When the wallpaper last checked whether the clock has moved on from its frames. */
        private long checked;

        private void tick() {
            handler.removeCallbacks(tick);
            if (!visible || destroyed) return;
            // A scene on the clock: bake it again for the new time of day every 20 minutes or so,
            // playing the old frames meanwhile.
            long now = SystemClock.uptimeMillis();
            if (player != null && baking == null && now - checked > 60_000) {
                checked = now;
                try {
                    Look look = Look.load(PathWallpaperService.this);
                    if (Bake.hoursStale(PathWallpaperService.this, look.scene()) > 0.33f) bakeHere();
                } catch (RuntimeException ignored) { }
            }
            draw();
            if (player == null) return;
            // On to the next frame when its turn comes.
            long frameMs = Math.max(8, player.frameNanos() / 1_000_000);
            long elapsed = SystemClock.uptimeMillis() - start;
            handler.postDelayed(tick, frameMs - elapsed % frameMs);
        }

        private void draw() {
            SurfaceHolder holder = getSurfaceHolder();
            Canvas c = null;
            try {
                try {
                    c = holder.lockHardwareCanvas();
                } catch (IllegalStateException | IllegalArgumentException e) {
                    c = holder.lockCanvas();
                }
                if (c == null) return;
                int w = c.getWidth(), h = c.getHeight();
                Bitmap b = null;
                if (player != null) {
                    long ns = (SystemClock.uptimeMillis() - start) * 1_000_000L;
                    b = player.frame((int) ((ns / player.frameNanos()) % player.frames));
                }
                if (b != null) {
                    Frame.cover(c, b, w, h, paint, matrix);
                } else {
                    c.drawColor(Color.rgb(18, 16, 28));
                    text.setTextSize(w * 0.04f);
                    c.drawText(status, w * 0.5f, h * 0.5f, text);
                }
            } finally {
                if (c != null) {
                    try { holder.unlockCanvasAndPost(c); } catch (IllegalStateException | IllegalArgumentException ignored) { }
                }
            }
        }
    }
}

package com.pathforge.mobile;

import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;

import java.io.BufferedOutputStream;
import java.io.ByteArrayOutputStream;
import java.io.Closeable;
import java.io.DataOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InterruptedIOException;
import java.io.RandomAccessFile;
import java.nio.ByteBuffer;

/** The live wallpaper's frames. A PathForge loop joins without a seam, so the wallpaper draws it
 *  once ("bakes" it) and then only plays it back: no rendering while it runs, little battery.
 *  The frames file is a header and one JPEG per frame. */
final class Bake {
    private static final int MAGIC = 0x50465731; // "PFW1"
    /** Per quality (Look.QUALITIES): the short side of the frames, and frames per second. */
    static final int[] SHORT_SIDE = {360, 540, 720};
    static final int[] FPS = {12, 20, 30};
    static final int MAX_FRAMES = 360;

    interface Progress {
        void at(int done, int total);
        boolean cancelled();
    }

    private Bake() {}

    static File file(Context c) { return new File(c.getFilesDir(), "wallpaper.frames"); }

    /** Draw the loop of `scene` for a screen of sw x sh at `quality` into the frames file. */
    static void bake(Context c, String scene, int quality, int sw, int sh, Progress p) throws IOException {
        long h = Native.open(scene, "");
        if (h == 0) throw new IOException(Native.lastError());
        try {
            float[] info = Native.info(h);
            float len = info[0], secs = info[1];
            int q = Math.max(0, Math.min(quality, FPS.length - 1));
            float k = Math.min(1f, SHORT_SIDE[q] / (float) Math.max(1, Math.min(sw, sh)));
            int w = Math.max(2, Math.round(sw * k)), hh = Math.max(2, Math.round(sh * k));
            int frames = Math.max(8, Math.min(MAX_FRAMES, Math.round(secs * FPS[q])));
            ByteBuffer buf = ByteBuffer.allocateDirect(w * hh * 4);
            Bitmap bmp = Bitmap.createBitmap(w, hh, Bitmap.Config.ARGB_8888);
            ByteArrayOutputStream jpg = new ByteArrayOutputStream(1 << 16);
            File tmp = new File(file(c).getPath() + ".tmp");
            try (DataOutputStream out = new DataOutputStream(new BufferedOutputStream(new FileOutputStream(tmp), 1 << 16))) {
                out.writeInt(MAGIC);
                out.writeInt(w);
                out.writeInt(hh);
                out.writeInt(frames);
                out.writeFloat(secs);
                out.writeBoolean(Look.pixelated(scene));
                for (int i = 0; i < frames; i++) {
                    if (p.cancelled()) throw new InterruptedIOException("cancelled");
                    // Frame i is i / frames of the way round the loop, in distance and in time.
                    if (!Native.render(h, len * i / frames, secs * i / frames, w, hh, buf)) throw new IOException(Native.lastError());
                    buf.rewind();
                    bmp.copyPixelsFromBuffer(buf);
                    buf.rewind();
                    jpg.reset();
                    bmp.compress(Bitmap.CompressFormat.JPEG, 92, jpg);
                    out.writeInt(jpg.size());
                    jpg.writeTo(out);
                    p.at(i + 1, frames);
                }
            } catch (IOException e) {
                //noinspection ResultOfMethodCallIgnored
                tmp.delete();
                throw e;
            }
            if (!tmp.renameTo(file(c))) throw new IOException("Could not save the wallpaper's frames");
        } finally {
            Native.close(h);
        }
    }

    /** Plays a frames file back, one frame decoded at a time into the same bitmap. */
    static final class Player implements Closeable {
        final int width, height, frames;
        final float seconds;
        final boolean pixel;
        private final RandomAccessFile f;
        private final long[] at;
        private final int[] size;
        private byte[] bytes = new byte[1 << 16];
        private Bitmap bmp;
        private final BitmapFactory.Options opts = new BitmapFactory.Options();

        private Player(RandomAccessFile f) throws IOException {
            this.f = f;
            if (f.readInt() != MAGIC) throw new IOException("not a frames file");
            width = f.readInt();
            height = f.readInt();
            frames = f.readInt();
            seconds = f.readFloat();
            pixel = f.readBoolean();
            if (width <= 0 || height <= 0 || frames <= 0 || frames > 10000 || !(seconds > 0)) throw new IOException("bad frames file");
            at = new long[frames];
            size = new int[frames];
            long pos = f.getFilePointer();
            for (int i = 0; i < frames; i++) {
                f.seek(pos);
                size[i] = f.readInt();
                at[i] = pos + 4;
                pos = at[i] + size[i];
            }
            if (pos > f.length()) throw new IOException("frames file cut short");
            opts.inMutable = true;
            opts.inPreferredConfig = Bitmap.Config.ARGB_8888;
        }

        static Player open(File file) throws IOException {
            RandomAccessFile f = new RandomAccessFile(file, "r");
            try {
                return new Player(f);
            } catch (IOException e) {
                f.close();
                throw e;
            }
        }

        /** How long each frame shows, in nanoseconds. */
        long frameNanos() { return (long) (seconds * 1e9 / frames); }

        /** Frame i (taken round the loop), or null if it cannot be read. */
        Bitmap frame(int i) {
            i = ((i % frames) + frames) % frames;
            try {
                if (bytes.length < size[i]) bytes = new byte[size[i] + (size[i] >> 2)];
                f.seek(at[i]);
                f.readFully(bytes, 0, size[i]);
                opts.inBitmap = bmp;
                Bitmap b;
                try {
                    b = BitmapFactory.decodeByteArray(bytes, 0, size[i], opts);
                } catch (IllegalArgumentException e) {
                    opts.inBitmap = null;
                    b = BitmapFactory.decodeByteArray(bytes, 0, size[i], opts);
                }
                if (b != null) bmp = b;
                return bmp;
            } catch (IOException e) {
                return bmp;
            }
        }

        @Override
        public void close() throws IOException { f.close(); }
    }

    /** Which bake the wallpaper should show: bumped by each new bake, so a running wallpaper
     *  picks the new frames up. */
    static int version(Context c) { return c.getSharedPreferences("wallpaper", Context.MODE_PRIVATE).getInt("version", 0); }

    static void bump(Context c) {
        android.content.SharedPreferences p = c.getSharedPreferences("wallpaper", Context.MODE_PRIVATE);
        p.edit().putInt("version", p.getInt("version", 0) + 1).commit();
    }
}

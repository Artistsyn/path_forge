package com.pathforge.mobile;

import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Matrix;
import android.graphics.Paint;

/** Drawing a rendered frame to fill a screen. */
final class Frame {
    private Frame() {}

    /** Draw `b` to cover a w x h canvas, centred, cut at the edges if its shape differs. */
    static void cover(Canvas c, Bitmap b, int w, int h, Paint paint, Matrix m) {
        float k = Math.max(w / (float) b.getWidth(), h / (float) b.getHeight());
        m.setScale(k, k);
        m.postTranslate((w - b.getWidth() * k) * 0.5f, (h - b.getHeight() * k) * 0.5f);
        c.drawBitmap(b, m, paint);
    }

    /** A paint that smooths the frame up to the screen, or keeps big pixels sharp. */
    static Paint paint(boolean pixel) {
        Paint p = new Paint();
        p.setFilterBitmap(!pixel);
        p.setDither(true);
        return p;
    }
}

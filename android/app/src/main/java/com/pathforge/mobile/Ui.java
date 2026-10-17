package com.pathforge.mobile;

import android.content.Context;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.widget.HorizontalScrollView;
import android.widget.LinearLayout;
import android.widget.TextView;

/** Small pieces of the app's look, built in code: chips, buttons, labels. */
final class Ui {
    static final int ACCENT = Color.rgb(255, 156, 84);
    static final int PANEL = Color.argb(236, 20, 17, 30);
    static final int TEXT = Color.rgb(240, 236, 244);
    static final int DIM = Color.rgb(170, 162, 184);

    private Ui() {}

    static int dp(Context c, float v) {
        return Math.round(TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, c.getResources().getDisplayMetrics()));
    }

    static GradientDrawable round(int fill, float radius, int stroke, int strokeWidth) {
        GradientDrawable d = new GradientDrawable();
        d.setColor(fill);
        d.setCornerRadius(radius);
        if (strokeWidth > 0) d.setStroke(strokeWidth, stroke);
        return d;
    }

    static TextView label(Context c, String s) {
        TextView t = new TextView(c);
        t.setText(s);
        t.setTextColor(DIM);
        t.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
        t.setAllCaps(true);
        t.setLetterSpacing(0.08f);
        t.setPadding(0, dp(c, 12), 0, dp(c, 6));
        return t;
    }

    /** A button: filled with the accent colour when `primary`, outlined otherwise. */
    static TextView button(Context c, String s, boolean primary, View.OnClickListener on) {
        TextView b = new TextView(c);
        b.setText(s);
        b.setGravity(Gravity.CENTER);
        b.setTextSize(TypedValue.COMPLEX_UNIT_SP, 14);
        b.setTypeface(Typeface.DEFAULT_BOLD);
        setPrimary(b, primary);
        b.setMinHeight(dp(c, 44));
        b.setPadding(dp(c, 14), dp(c, 10), dp(c, 14), dp(c, 10));
        b.setOnClickListener(on);
        b.setClickable(true);
        b.setFocusable(true);
        return b;
    }

    /** Fill a button with the accent colour (or outline it again). */
    static void setPrimary(TextView b, boolean primary) {
        Context c = b.getContext();
        b.setTextColor(primary ? Color.rgb(30, 18, 10) : TEXT);
        b.setBackground(round(primary ? ACCENT : Color.TRANSPARENT, dp(c, 22), Color.argb(110, 255, 255, 255), primary ? 0 : dp(c, 1)));
    }

    interface Pick { void at(int i); }

    /** A scrolling row of choices, one selected. */
    static final class Chips {
        final HorizontalScrollView view;
        final LinearLayout row;
        private final TextView[] items;
        private int selected;

        Chips(Context c, String[] labels, int selected, Pick pick) {
            view = new HorizontalScrollView(c);
            view.setHorizontalScrollBarEnabled(false);
            row = new LinearLayout(c);
            row.setOrientation(LinearLayout.HORIZONTAL);
            view.addView(row);
            items = new TextView[labels.length];
            for (int i = 0; i < labels.length; i++) {
                final int k = i;
                TextView t = new TextView(c);
                t.setText(labels[i]);
                t.setTextSize(TypedValue.COMPLEX_UNIT_SP, 14);
                t.setGravity(Gravity.CENTER);
                t.setMinHeight(dp(c, 40));
                t.setPadding(dp(c, 16), dp(c, 8), dp(c, 16), dp(c, 8));
                t.setOnClickListener(v -> { select(k); pick.at(k); });
                LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT);
                lp.rightMargin = dp(c, 8);
                row.addView(t, lp);
                items[i] = t;
            }
            select(selected);
        }

        void select(int i) {
            selected = i;
            for (int k = 0; k < items.length; k++) {
                boolean on = k == i;
                Context c = items[k].getContext();
                items[k].setTextColor(on ? Color.rgb(30, 18, 10) : TEXT);
                items[k].setBackground(round(on ? ACCENT : Color.argb(40, 255, 255, 255), dp(c, 20), 0, 0));
            }
        }

        int selected() { return selected; }
    }
}

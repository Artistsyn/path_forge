package com.pathforge.mobile;

import android.app.AlertDialog;
import android.content.Context;
import android.graphics.Color;
import android.graphics.drawable.GradientDrawable;
import android.text.Editable;
import android.text.TextWatcher;
import android.util.TypedValue;
import android.view.View;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.SeekBar;
import android.widget.TextView;

/** A colour chooser: hue, saturation and brightness sliders, and the hex code. */
final class ColorPicker {
    interface Picked { void color(int rgb); }

    private ColorPicker() {}

    static void show(Context c, String title, int start, Picked picked) {
        final float[] hsv = new float[3];
        Color.colorToHSV(start, hsv);
        LinearLayout box = new LinearLayout(c);
        box.setOrientation(LinearLayout.VERTICAL);
        int p = Ui.dp(c, 20);
        box.setPadding(p, p / 2, p, 0);
        View swatch = new View(c);
        box.addView(swatch, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, Ui.dp(c, 56)));
        EditText hex = new EditText(c);
        hex.setSingleLine(true);
        hex.setTextSize(TypedValue.COMPLEX_UNIT_SP, 16);
        final boolean[] quiet = {false};
        final SeekBar[] bars = new SeekBar[3];
        Runnable show = () -> {
            int col = Color.HSVToColor(hsv);
            GradientDrawable d = Ui.round(col, Ui.dp(c, 12), Color.argb(80, 255, 255, 255), Ui.dp(c, 1));
            swatch.setBackground(d);
            quiet[0] = true;
            hex.setText(String.format("#%06X", col & 0xFFFFFF));
            quiet[0] = false;
        };
        String[] names = {"Hue", "Saturation", "Brightness"};
        int[] max = {360, 100, 100};
        for (int i = 0; i < 3; i++) {
            final int k = i;
            TextView l = new TextView(c);
            l.setText(names[i]);
            l.setPadding(0, Ui.dp(c, 10), 0, 0);
            box.addView(l);
            SeekBar s = new SeekBar(c);
            s.setMax(max[i]);
            s.setProgress(Math.round(k == 0 ? hsv[0] : hsv[k] * 100));
            s.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
                @Override public void onProgressChanged(SeekBar b, int v, boolean user) {
                    if (!user) return;
                    hsv[k] = k == 0 ? v : v / 100f;
                    show.run();
                }
                @Override public void onStartTrackingTouch(SeekBar b) { }
                @Override public void onStopTrackingTouch(SeekBar b) { }
            });
            bars[i] = s;
            box.addView(s);
        }
        box.addView(hex);
        hex.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence s, int a, int b, int n) { }
            @Override public void onTextChanged(CharSequence s, int a, int b, int n) { }
            @Override public void afterTextChanged(Editable e) {
                if (quiet[0]) return;
                try {
                    int col = Color.parseColor(e.toString().trim().startsWith("#") ? e.toString().trim() : "#" + e.toString().trim());
                    Color.colorToHSV(col, hsv);
                    bars[0].setProgress(Math.round(hsv[0]));
                    bars[1].setProgress(Math.round(hsv[1] * 100));
                    bars[2].setProgress(Math.round(hsv[2] * 100));
                    swatch.setBackground(Ui.round(col, Ui.dp(c, 12), Color.argb(80, 255, 255, 255), Ui.dp(c, 1)));
                } catch (IllegalArgumentException ignored) {
                }
            }
        });
        show.run();
        new AlertDialog.Builder(c).setTitle(title).setView(box)
                .setPositiveButton("OK", (d, w) -> picked.color(Color.HSVToColor(hsv) & 0xFFFFFF))
                .setNegativeButton("Cancel", null).show();
    }
}

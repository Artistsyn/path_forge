package com.pathforge.mobile;

import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.os.BatteryManager;
import android.os.PowerManager;

/** The live wallpaper's battery saver: fewer frames a second while the battery is low or the
 *  phone is saving power (or always, or never). The loop still plays at its own speed; frames
 *  in between are skipped. */
final class Saver {
    static final String[] MODES = {"When low", "Always", "Off"};

    private Saver() {}

    static int mode(Context c) { return c.getSharedPreferences("look", Context.MODE_PRIVATE).getInt("saver", 0); }
    static void setMode(Context c, int m) { c.getSharedPreferences("look", Context.MODE_PRIVATE).edit().putInt("saver", m).apply(); }

    /** How many of the loop's frames each drawn frame stands for: 1 at full rate, 2 at half, 3
     *  when the battery is nearly flat. */
    static int stride(Context c) {
        int m = mode(c);
        if (m == 2) return 1;
        if (m == 1) return 2;
        PowerManager pm = (PowerManager) c.getSystemService(Context.POWER_SERVICE);
        boolean saving = pm != null && pm.isPowerSaveMode();
        Intent b = c.registerReceiver(null, new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
        int pct = 100;
        boolean charging = false;
        if (b != null) {
            int level = b.getIntExtra(BatteryManager.EXTRA_LEVEL, -1), scale = b.getIntExtra(BatteryManager.EXTRA_SCALE, 100);
            if (level >= 0 && scale > 0) pct = 100 * level / scale;
            charging = b.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) != 0;
        }
        if (charging) return saving ? 2 : 1;
        if (pct <= 10) return 3;
        return saving || pct <= 20 ? 2 : 1;
    }
}

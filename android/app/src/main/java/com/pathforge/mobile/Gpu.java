package com.pathforge.mobile;

import android.content.Context;
import android.content.SharedPreferences;

/** Whether to draw on the phone's GPU, and a guard against a GPU driver that crashes the app
 *  (which no code can catch): the first GPU frame of a run is announced before and confirmed after,
 *  so a run that died in between is found on the next start, and the app then stays on the CPU. */
final class Gpu {
    private static volatile boolean proven;

    private Gpu() {}

    private static SharedPreferences prefs(Context c) { return c.getSharedPreferences("gpu", Context.MODE_PRIVATE); }

    static boolean wanted(Context c) { return prefs(c).getBoolean("on", true); }

    static void setWanted(Context c, boolean on) { prefs(c).edit().putBoolean("on", on).putBoolean("pending", false).apply(); }

    /** True (once) if the last run died while starting the GPU: the GPU is then switched off. */
    static boolean crashedLastTime(Context c) {
        SharedPreferences p = prefs(c);
        if (!p.getBoolean("pending", false)) return false;
        p.edit().putBoolean("pending", false).putBoolean("on", false).commit();
        return true;
    }

    /** Open a scene on the GPU if it is wanted (see Native.open). */
    static long open(Context c, String json) {
        boolean gpu = wanted(c);
        if (gpu && !proven) prefs(c).edit().putBoolean("pending", true).commit();
        long h = Native.open(json, "", gpu);
        if (gpu && h != 0 && !Native.engine(h).equals("gpu")) settled(c);
        return h;
    }

    /** After a frame of handle h: once one GPU frame has been drawn, the GPU is known to work. */
    static void rendered(Context c, long h) {
        if (!proven && Native.engine(h).equals("gpu")) settled(c);
    }

    private static void settled(Context c) {
        proven = true;
        prefs(c).edit().putBoolean("pending", false).commit();
    }
}

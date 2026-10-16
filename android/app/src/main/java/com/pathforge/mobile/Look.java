package com.pathforge.mobile;

import android.content.Context;
import android.content.SharedPreferences;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;

/** The scene the phone is working on (a preset, one of yours, or a file from the desktop), kept
 *  between runs; the wallpaper reads the same. The quick controls here change the scene itself,
 *  as the editor does, so what is saved is what is shown. */
final class Look {
    static final String[] SKIES = {"As designed", "Golden", "Fiery", "Pastel", "Dusk"};
    static final String[] CLOUDS = {"As designed", "Mixed"};
    static final String[] SEASONS = {"As designed", "Spring", "Summer", "Autumn", "Winter", "Year cycle"};
    static final String[] TIMES = {"As designed", "Real time", "Dawn", "Noon", "Dusk", "Night", "Day cycle"};
    /** The hour each of TIMES' fixed times stands at. */
    private static final double[] HOURS = {-1, -1, 6.4, 12, 18.1, 23, 5};
    static final String[] QUALITIES = {"Battery saver", "Balanced", "Smooth"};
    static final String DEFAULT_PRESET = "Sunset Pier";

    /** What it is called, and whether it is one of the scenes you saved under that name. */
    String name;
    boolean mine;
    /** The scene, as JSON. */
    String scene;
    /** Index into QUALITIES, for the live wallpaper. */
    int quality = 1;

    private static SharedPreferences prefs(Context c) { return c.getSharedPreferences("look", Context.MODE_PRIVATE); }
    private static File sceneFile(Context c) { return new File(c.getFilesDir(), "scene.json"); }

    static Look load(Context c) {
        Look l = new Look();
        SharedPreferences p = prefs(c);
        l.name = p.getString("name", DEFAULT_PRESET);
        l.mine = p.getBoolean("mine", false);
        l.quality = p.getInt("quality", 1);
        try { l.scene = read(new FileInputStream(sceneFile(c))); } catch (IOException e) { l.scene = null; }
        if (l.scene == null) {
            l.name = DEFAULT_PRESET;
            l.mine = false;
            l.scene = Native.presetScene(DEFAULT_PRESET);
        }
        return l;
    }

    void save(Context c) {
        prefs(c).edit().putString("name", name).putBoolean("mine", mine).putInt("quality", quality).apply();
        try (FileOutputStream out = new FileOutputStream(sceneFile(c))) {
            out.write(scene.getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {
        }
    }

    String scene() { return scene; }

    // ── Quick controls ──────────────────────────────────────────────────

    private JSONObject parsed() {
        try { return new JSONObject(scene); } catch (JSONException e) { return new JSONObject(); }
    }

    private static JSONObject obj(JSONObject parent, String key) throws JSONException {
        JSONObject o = parent.optJSONObject(key);
        if (o == null) { o = new JSONObject(); parent.put(key, o); }
        return o;
    }

    /** Index into SKIES of the scene's sunset (0: none). */
    int sky() {
        JSONObject ss = parsed().optJSONObject("sky");
        ss = ss == null ? null : ss.optJSONObject("sunset");
        if (ss == null || !ss.optBoolean("enabled")) return 0;
        String style = ss.optString("style", "Golden");
        for (int i = 1; i < SKIES.length; i++) if (SKIES[i].equals(style)) return i;
        return 1;
    }

    /** Give the scene the sunset SKIES[i], the sun brought down low (and set at dusk); 0 takes it away. */
    void setSky(int i) {
        try {
            JSONObject s = parsed();
            JSONObject sky = obj(s, "sky");
            JSONObject ss = obj(sky, "sunset");
            ss.put("enabled", i > 0);
            if (i > 0) {
                ss.put("style", SKIES[i]);
                JSONObject sun = obj(sky, "sun");
                JSONArray pos = sun.optJSONArray("pos");
                if (pos == null || pos.length() < 2) { pos = new JSONArray().put(0.6).put(0.9); sun.put("pos", pos); }
                if (pos.optDouble(1, 0) < 0.8) pos.put(1, 0.88);
                boolean dusk = i == 4;
                sun.put("enabled", !dusk);
                if (!dusk) {
                    obj(sky, "stars").put("enabled", false);
                    // A night scene lit as an evening.
                    JSONObject light = obj(s, "light");
                    light.put("ambient", Math.max(light.optDouble("ambient", 0.6), 0.35));
                }
            }
            scene = s.toString();
        } catch (JSONException ignored) {
        }
    }

    /** Index into TIMES of the scene's day and night (0: none). */
    int time() {
        JSONObject sky = parsed().optJSONObject("sky");
        JSONObject dn = sky == null ? null : sky.optJSONObject("day_night");
        if (dn == null || !dn.optBoolean("enabled")) return 0;
        if (dn.optBoolean("real_time")) return 1;
        if (dn.optInt("days_per_loop", 0) > 0) return 6;
        double h = dn.optDouble("hour", 12);
        int best = 3;
        for (int i = 2; i < 6; i++) if (Math.abs(HOURS[i] - h) < Math.abs(HOURS[best] - h)) best = i;
        return best;
    }

    /** Set the scene's time of day to TIMES[i]: the clock, a fixed hour, or a day each loop; 0 as designed. */
    void setTime(int i) {
        try {
            JSONObject s = parsed();
            JSONObject dn = obj(obj(s, "sky"), "day_night");
            dn.put("enabled", i > 0);
            dn.put("real_time", i == 1);
            dn.put("days_per_loop", i == 6 ? 1 : 0);
            if (i >= 2) dn.put("hour", HOURS[i]);
            scene = s.toString();
        } catch (JSONException ignored) {
        }
    }

    /** Index into SEASONS of the scene's season (0: none). */
    int season() {
        JSONObject se = parsed().optJSONObject("season");
        if (se == null || !se.optBoolean("enabled")) return 0;
        if (se.optInt("years_per_loop", 0) > 0) return 5;
        return 1 + (int) Math.round(se.optDouble("season", 1)) % 4;
    }

    /** Set the season to SEASONS[i] (a year each loop for the last); 0 as designed. */
    void setSeason(int i) {
        try {
            JSONObject s = parsed();
            JSONObject se = obj(s, "season");
            se.put("enabled", i > 0);
            se.put("years_per_loop", i == 5 ? 1 : 0);
            if (i >= 1 && i <= 4) se.put("season", i - 1);
            if (i == 5) se.put("season", 0);
            scene = s.toString();
        } catch (JSONException ignored) {
        }
    }

    boolean mixed() {
        JSONObject sky = parsed().optJSONObject("sky");
        JSONObject mix = sky == null ? null : sky.optJSONObject("cloud_mix");
        return mix != null && mix.optBoolean("enabled");
    }

    /** A random mix of cloud kinds (`seed` >= 0 deals another), or none. */
    void setMixed(boolean on, int seed) {
        try {
            JSONObject s = parsed();
            JSONObject mix = obj(obj(s, "sky"), "cloud_mix");
            mix.put("enabled", on);
            if (on && !mix.has("layers")) mix.put("layers", 3);
            if (on && mix.optDouble("patches", 0) <= 0) mix.put("patches", 0.5);
            if (seed >= 0) mix.put("seed", seed);
            scene = s.toString();
        } catch (JSONException ignored) {
        }
    }

    /** Walking speed, metres a second. */
    double speed() {
        JSONObject m = parsed().optJSONObject("motion");
        return m == null ? 2.5 : m.optDouble("speed", 2.5);
    }

    void setSpeed(double v) {
        try {
            JSONObject s = parsed();
            obj(s, "motion").put("speed", v);
            scene = s.toString();
        } catch (JSONException ignored) {
        }
    }

    /** Whether the sky is drawn (dungeons and caves have none, so sky controls do nothing there). */
    boolean hasSky() {
        JSONObject sky = parsed().optJSONObject("sky");
        return sky == null || sky.optBoolean("enabled", true);
    }

    /** Whether a scene's day and night follow the phone's clock (sky.day_night.real_time). */
    static boolean followsClock(String scene) {
        try {
            JSONObject sky = new JSONObject(scene).optJSONObject("sky");
            JSONObject dn = sky == null ? null : sky.optJSONObject("day_night");
            return dn != null && dn.optBoolean("enabled", false) && dn.optBoolean("real_time", false);
        } catch (JSONException e) {
            return false;
        }
    }

    /** Whether a scene draws in big pixels (then it is scaled up without smoothing). */
    static boolean pixelated(String scene) {
        try {
            JSONObject style = new JSONObject(scene).optJSONObject("style");
            return style != null && style.optInt("pixel_size", 1) > 1;
        } catch (JSONException e) {
            return false;
        }
    }

    static String read(InputStream in) throws IOException {
        try (InputStream i = in) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] b = new byte[1 << 14];
            for (int n; (n = i.read(b)) > 0; ) out.write(b, 0, n);
            return out.toString("UTF-8");
        }
    }
}

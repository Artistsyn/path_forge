package com.pathforge.mobile;

import android.content.Context;
import android.content.SharedPreferences;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.ByteArrayOutputStream;
import java.nio.charset.StandardCharsets;

/** The scene the phone is working on (a preset, or a scene file made on the desktop) and the quick
 *  changes made to it here, kept between runs. The wallpaper reads the same. */
final class Look {
    static final String[] SKIES = {"As designed", "Golden", "Fiery", "Pastel", "Dusk"};
    static final String[] CLOUDS = {"As designed", "Mixed"};
    static final String[] QUALITIES = {"Battery saver", "Balanced", "Smooth"};
    static final String DEFAULT_PRESET = "Sunset Pier";

    String name;
    /** The scene as chosen, before the changes below. */
    String base;
    /** Index into SKIES: 0 leaves the sky alone, the rest give it that sunset. */
    int sky;
    boolean mixClouds;
    int cloudSeed;
    /** How much faster than designed the walk goes. */
    float speed = 1f;
    /** Index into QUALITIES, for the live wallpaper. */
    int quality = 1;

    private static SharedPreferences prefs(Context c) { return c.getSharedPreferences("look", Context.MODE_PRIVATE); }
    private static File sceneFile(Context c) { return new File(c.getFilesDir(), "scene.json"); }

    static Look load(Context c) {
        Look l = new Look();
        SharedPreferences p = prefs(c);
        l.name = p.getString("name", DEFAULT_PRESET);
        l.sky = p.getInt("sky", 0);
        l.mixClouds = p.getBoolean("mix", false);
        l.cloudSeed = p.getInt("seed", 7);
        l.speed = p.getFloat("speed", 1f);
        l.quality = p.getInt("quality", 1);
        try { l.base = read(new FileInputStream(sceneFile(c))); } catch (IOException e) { l.base = null; }
        if (l.base == null) { l.name = DEFAULT_PRESET; l.base = Native.presetScene(DEFAULT_PRESET); }
        return l;
    }

    void save(Context c) {
        prefs(c).edit().putString("name", name).putInt("sky", sky).putBoolean("mix", mixClouds).putInt("seed", cloudSeed)
                .putFloat("speed", speed).putInt("quality", quality).apply();
        try (FileOutputStream out = new FileOutputStream(sceneFile(c))) {
            out.write(base.getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {
        }
    }

    /** The scene with the phone's changes made. */
    String scene() {
        try {
            JSONObject s = new JSONObject(base);
            JSONObject sky = s.optJSONObject("sky");
            if (sky != null && sky.optBoolean("enabled", true)) {
                if (this.sky > 0) sunset(s, sky, SKIES[this.sky]);
                if (mixClouds) {
                    JSONObject mix = obj(sky, "cloud_mix");
                    mix.put("enabled", true);
                    mix.put("layers", 3);
                    mix.put("patches", 0.5);
                    mix.put("seed", cloudSeed);
                }
            }
            JSONObject motion = obj(s, "motion");
            motion.put("speed", motion.optDouble("speed", 2.5) * speed);
            return s.toString();
        } catch (JSONException e) {
            return base;
        }
    }

    /** A sunset of `style`, the sun brought down low (and set, at dusk). */
    private static void sunset(JSONObject s, JSONObject sky, String style) throws JSONException {
        JSONObject ss = obj(sky, "sunset");
        ss.put("enabled", true);
        ss.put("style", style);
        JSONObject sun = obj(sky, "sun");
        JSONArray pos = sun.optJSONArray("pos");
        if (pos == null || pos.length() < 2) { pos = new JSONArray().put(0.6).put(0.9); sun.put("pos", pos); }
        if (pos.optDouble(1, 0) < 0.8) pos.put(1, 0.88);
        boolean dusk = style.equals("Dusk");
        sun.put("enabled", !dusk);
        if (!dusk) {
            obj(sky, "stars").put("enabled", false);
            // A night scene lit as an evening.
            JSONObject light = obj(s, "light");
            light.put("ambient", Math.max(light.optDouble("ambient", 0.6), 0.35));
        }
    }

    private static JSONObject obj(JSONObject parent, String key) throws JSONException {
        JSONObject o = parent.optJSONObject(key);
        if (o == null) { o = new JSONObject(); parent.put(key, o); }
        return o;
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

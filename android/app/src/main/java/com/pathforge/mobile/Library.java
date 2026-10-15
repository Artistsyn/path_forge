package com.pathforge.mobile;

import android.content.Context;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/** Your own scenes: one JSON file each, named after the scene, newest first. */
final class Library {
    private Library() {}

    private static File dir(Context c) {
        File d = new File(c.getFilesDir(), "scenes");
        //noinspection ResultOfMethodCallIgnored
        d.mkdirs();
        return d;
    }

    /** A name as a file name: what a file system cannot hold is replaced. */
    private static File file(Context c, String name) {
        return new File(dir(c), name.replaceAll("[\\\\/:*?\"<>|\\x00-\\x1f]", "_") + ".json");
    }

    static List<String> names(Context c) {
        File[] fs = dir(c).listFiles((d, n) -> n.endsWith(".json"));
        if (fs == null) return new ArrayList<>();
        Arrays.sort(fs, (a, b) -> Long.compare(b.lastModified(), a.lastModified()));
        List<String> out = new ArrayList<>();
        for (File f : fs) out.add(f.getName().substring(0, f.getName().length() - 5));
        return out;
    }

    static boolean exists(Context c, String name) { return file(c, name).exists(); }

    static long modified(Context c, String name) { return file(c, name).lastModified(); }

    static String load(Context c, String name) {
        try { return Look.read(new FileInputStream(file(c, name))); } catch (IOException e) { return null; }
    }

    static boolean save(Context c, String name, String scene) {
        File f = file(c, name), tmp = new File(f.getPath() + ".tmp");
        try (FileOutputStream out = new FileOutputStream(tmp)) {
            out.write(scene.getBytes(StandardCharsets.UTF_8));
        } catch (IOException e) {
            return false;
        }
        return tmp.renameTo(f);
    }

    static boolean delete(Context c, String name) { return file(c, name).delete(); }

    static boolean rename(Context c, String from, String to) { return file(c, from).renameTo(file(c, to)); }
}

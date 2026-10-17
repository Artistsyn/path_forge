package com.pathforge.mobile;

import android.app.Activity;
import android.app.AlertDialog;
import android.app.WallpaperInfo;
import android.app.WallpaperManager;
import android.content.ActivityNotFoundException;
import android.content.ComponentName;
import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.SharedPreferences;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Color;
import android.graphics.Point;
import android.graphics.Rect;
import android.graphics.Typeface;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.os.Handler;
import android.os.Looper;
import android.provider.MediaStore;
import android.util.DisplayMetrics;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowInsets;
import android.widget.FrameLayout;
import android.widget.HorizontalScrollView;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.ScrollView;
import android.widget.SeekBar;
import android.widget.EditText;
import android.widget.TextView;
import android.widget.Toast;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;

/** The app: the scene walking past full screen, and under it a panel to pick a scene, give it a
 *  sunset or a mixed sky, set the pace, and make it the phone's wallpaper (live, or a still for the
 *  home and lock screens). Tap the scene to hide the panel. */
public final class MainActivity extends Activity implements Editor.Host {
    private static final int PICK_SCENE = 1;
    private static final int EXPORT_SCENE = 2;

    private Look look;
    private PreviewView preview;
    private LinearLayout panel;
    private Editor editor;
    private TextView title;
    private TextView status;
    private TextView speedLabel;
    private SeekBar speedBar;
    private TextView showButton;
    private Ui.Chips skyChips, timeChips, seasonChips, cloudChips;
    private LinearLayout strip;
    private int bottomInset, topInset;
    /** The strip's tiles: (name, one of yours?, its picture). */
    private final List<Object[]> tiles = new ArrayList<>();
    private String[] presets = new String[0];
    /** A scene waiting for the file it is to be exported into. */
    private String exporting;
    private final Handler ui = new Handler(Looper.getMainLooper());
    private final ExecutorService work = Executors.newSingleThreadExecutor();
    private final ExecutorService thumbs = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "pathforge-thumbs");
        t.setPriority(Thread.MIN_PRIORITY);
        return t;
    });
    /** The GPU's name once it has started, "" if there is none, null until then. */
    private volatile String gpuName;
    private final Runnable applyEdit = () -> { look.save(this); preview.setScene(look.scene()); };

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        edgeToEdge();
        look = Look.load(this);
        try {
            JSONArray a = new JSONArray(Native.presetNames());
            presets = new String[a.length()];
            for (int i = 0; i < a.length(); i++) presets[i] = a.getString(i);
        } catch (JSONException ignored) { }

        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(Color.BLACK);
        preview = new PreviewView(this);
        preview.setCanvasShape(getSharedPreferences("look", MODE_PRIVATE).getBoolean("canvas_shape", true));
        preview.setOnClickListener(v -> { if (editor.view.getVisibility() != View.VISIBLE) showPanel(panel.getVisibility() != View.VISIBLE); });
        root.addView(preview, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));

        panel = buildPanel();
        root.addView(panel, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM));

        editor = new Editor(this, Native.form(), this);
        editor.view.setVisibility(View.GONE);
        int editorHeight = Math.round(getResources().getDisplayMetrics().heightPixels * 0.6f);
        root.addView(editor.view, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, editorHeight, Gravity.BOTTOM));

        showButton = Ui.button(this, "Edit", false, v -> showPanel(true));
        showButton.setBackground(Ui.round(Ui.PANEL, Ui.dp(this, 22), Color.argb(110, 255, 255, 255), Ui.dp(this, 1)));
        showButton.setVisibility(View.GONE);
        root.addView(showButton, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM | Gravity.END));

        root.setOnApplyWindowInsetsListener((v, in) -> {
            if (Build.VERSION.SDK_INT >= 30) {
                android.graphics.Insets bars = in.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.ime());
                bottomInset = bars.bottom;
                topInset = bars.top;
            } else {
                bottomInset = in.getSystemWindowInsetBottom();
                topInset = in.getSystemWindowInsetTop();
            }
            int p = Ui.dp(this, 16);
            panel.setPadding(p, p, p, p + bottomInset);
            editor.view.setPadding(p, p, p, bottomInset);
            FrameLayout.LayoutParams lp = (FrameLayout.LayoutParams) showButton.getLayoutParams();
            lp.setMargins(p, p, p, p + bottomInset);
            showButton.setLayoutParams(lp);
            return in;
        });
        setContentView(root);
        if (Gpu.crashedLastTime(this)) toast("The GPU stopped PathForge last time, so it draws on the CPU now. You can switch back under Draw on.");
        preview.setStats((engine, w, h, fps) -> ui.post(() -> status.setText(String.format(java.util.Locale.US,
                "%s  ·  %d×%d  ·  %.0f fps", engine.equals("gpu") ? "GPU" + (gpuName == null || gpuName.isEmpty() ? "" : " " + gpuName) : "CPU", w, h, fps))));
        if (Gpu.wanted(this)) work.execute(() -> gpuName = Native.gpuInit());
        refreshStrip();
        sceneChanged(true);
    }

    private void edgeToEdge() {
        Window w = getWindow();
        w.setStatusBarColor(Color.TRANSPARENT);
        w.setNavigationBarColor(Color.TRANSPARENT);
        if (Build.VERSION.SDK_INT >= 30) {
            w.setDecorFitsSystemWindows(false);
        } else {
            w.getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_LAYOUT_STABLE | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
    }

    private View gap(int dp) {
        View g = new View(this);
        g.setLayoutParams(new LinearLayout.LayoutParams(Ui.dp(this, dp), 1));
        return g;
    }

    private LinearLayout buildPanel() {
        LinearLayout p = new LinearLayout(this);
        p.setOrientation(LinearLayout.VERTICAL);
        float r = Ui.dp(this, 24);
        android.graphics.drawable.GradientDrawable bg = new android.graphics.drawable.GradientDrawable();
        bg.setColor(Ui.PANEL);
        bg.setCornerRadii(new float[]{r, r, r, r, 0, 0, 0, 0});
        p.setBackground(bg);
        p.setClickable(true);

        // The scene's name, then Import and Hide.
        LinearLayout head = new LinearLayout(this);
        head.setGravity(Gravity.CENTER_VERTICAL);
        title = new TextView(this);
        title.setTextColor(Ui.TEXT);
        title.setTextSize(TypedValue.COMPLEX_UNIT_SP, 20);
        title.setTypeface(Typeface.DEFAULT_BOLD);
        title.setSingleLine(true);
        title.setEllipsize(android.text.TextUtils.TruncateAt.END);
        head.addView(title, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        head.addView(Ui.button(this, "Import", false, v -> pickScene()));
        head.addView(gap(8));
        head.addView(Ui.button(this, "Hide", false, v -> showPanel(false)));
        p.addView(head);
        status = new TextView(this);
        status.setTextColor(Ui.DIM);
        status.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
        status.setSingleLine(true);
        status.setEllipsize(android.text.TextUtils.TruncateAt.END);
        p.addView(status);

        // Everything else scrolls, so it fits a short screen.
        ScrollView scroll = new ScrollView(this);
        scroll.setVerticalScrollBarEnabled(false);
        LinearLayout body = new LinearLayout(this);
        body.setOrientation(LinearLayout.VERTICAL);
        scroll.addView(body);
        DisplayMetrics dm = getResources().getDisplayMetrics();
        p.addView(scroll, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        scroll.post(() -> {
            int max = (int) (dm.heightPixels * 0.36f);
            if (scroll.getHeight() > max) scroll.setLayoutParams(new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, max));
        });

        body.addView(Ui.label(this, "Scenes  ·  long-press yours to rename, copy, export or delete"));
        HorizontalScrollView sv = new HorizontalScrollView(this);
        sv.setHorizontalScrollBarEnabled(false);
        strip = new LinearLayout(this);
        sv.addView(strip);
        body.addView(sv);

        body.addView(Ui.label(this, "Sunset"));
        skyChips = new Ui.Chips(this, Look.SKIES, look.sky(), i -> {
            if (!look.hasSky()) { toast("This scene has no sky (switch it on under Edit → Sky)"); skyChips.select(0); return; }
            look.setSky(i);
            sceneChanged(false);
        });
        body.addView(skyChips.view);

        body.addView(Ui.label(this, "Time of day"));
        timeChips = new Ui.Chips(this, Look.TIMES, look.time(), i -> {
            if (!look.hasSky()) { toast("This scene has no sky (switch it on under Edit → Sky)"); timeChips.select(0); return; }
            look.setTime(i);
            if (i == 1) toast("Follows your phone's clock; the wallpaper redraws as the hours pass");
            sceneChanged(false);
        });
        body.addView(timeChips.view);

        body.addView(Ui.label(this, "Season"));
        seasonChips = new Ui.Chips(this, Look.SEASONS, look.season(), i -> { look.setSeason(i); sceneChanged(false); });
        body.addView(seasonChips.view);

        body.addView(Ui.label(this, "Clouds"));
        LinearLayout clouds = new LinearLayout(this);
        clouds.setGravity(Gravity.CENTER_VERTICAL);
        cloudChips = new Ui.Chips(this, Look.CLOUDS, look.mixed() ? 1 : 0, i -> { look.setMixed(i == 1, -1); sceneChanged(false); });
        clouds.addView(cloudChips.view);
        clouds.addView(Ui.button(this, "Shuffle", false, v -> {
            look.setMixed(true, (int) (Math.random() * 9999));
            cloudChips.select(1);
            sceneChanged(false);
        }));
        body.addView(clouds);

        speedLabel = Ui.label(this, "");
        body.addView(speedLabel);
        speedBar = new SeekBar(this);
        speedBar.setMax(115);
        speedBar.getProgressDrawable().setTint(Ui.ACCENT);
        speedBar.getThumb().setTint(Ui.ACCENT);
        speedBar.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            @Override public void onProgressChanged(SeekBar s, int v, boolean user) {
                if (!user) return;
                look.setSpeed(Math.round((0.5 + v * 0.1) * 10) / 10.0);
                showSpeed();
            }
            @Override public void onStartTrackingTouch(SeekBar s) { }
            @Override public void onStopTrackingTouch(SeekBar s) { sceneChanged(false); }
        });
        body.addView(speedBar, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, Ui.dp(this, 40)));

        body.addView(Ui.label(this, "Live wallpaper"));
        body.addView(new Ui.Chips(this, Look.QUALITIES, look.quality, i -> { look.quality = i; look.save(this); }).view);
        body.addView(Ui.label(this, "Wallpaper battery saver"));
        body.addView(new Ui.Chips(this, Saver.MODES, Saver.mode(this), i -> {
            Saver.setMode(this, i);
            if (i == 0) toast("Fewer frames a second while the battery is low or the phone saves power");
        }).view);

        body.addView(Ui.label(this, "Draw on"));
        body.addView(new Ui.Chips(this, new String[]{"GPU (fast)", "CPU"}, Gpu.wanted(this) ? 0 : 1, i -> {
            Gpu.setWanted(this, i == 0);
            if (i == 0 && (gpuName == null || gpuName.isEmpty())) work.execute(() -> gpuName = Native.gpuInit());
            preview.reopen();
        }).view);

        body.addView(Ui.label(this, "Preview"));
        boolean sharp = getSharedPreferences("look", MODE_PRIVATE).getBoolean("sharp", false);
        preview.setSharp(sharp);
        body.addView(new Ui.Chips(this, new String[]{"Auto (smooth)", "Sharp (full size)"}, sharp ? 1 : 0, i -> {
            getSharedPreferences("look", MODE_PRIVATE).edit().putBoolean("sharp", i == 1).apply();
            preview.setSharp(i == 1);
        }).view);
        body.addView(Ui.label(this, "Shape"));
        shapeChips = new Ui.Chips(this, new String[]{"The canvas's", "The whole screen"}, canvasPreview() ? 0 : 1, i -> setCanvasPreview(i == 0));
        body.addView(shapeChips.view);

        // Every setting, and keeping it.
        LinearLayout edit = new LinearLayout(this);
        edit.setPadding(0, Ui.dp(this, 14), 0, 0);
        edit.addView(Ui.button(this, "Edit every setting", false, v -> openEditor()), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.4f));
        edit.addView(gap(8));
        edit.addView(Ui.button(this, "Save scene", false, v -> askSave(look.name, null)), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        p.addView(edit);

        // A new scene at random (keeping the parts you lock), and scenes as share codes.
        LinearLayout fun = new LinearLayout(this);
        fun.setPadding(0, Ui.dp(this, 8), 0, 0);
        TextView surprise = Ui.button(this, "Surprise me", false, v -> surprise());
        surprise.setOnLongClickListener(v -> { askLocks(); return true; });
        fun.addView(surprise, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.3f));
        fun.addView(gap(6));
        fun.addView(Ui.button(this, "Locks", false, v -> askLocks()), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 0.8f));
        fun.addView(gap(6));
        backButton = Ui.button(this, "Back", false, v -> back());
        backButton.setAlpha(0.4f);
        fun.addView(backButton, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 0.8f));
        p.addView(fun);
        LinearLayout codes = new LinearLayout(this);
        codes.setPadding(0, Ui.dp(this, 8), 0, 0);
        codes.addView(Ui.button(this, "Share code / QR", false, v -> shareCode()), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.2f));
        codes.addView(gap(6));
        codes.addView(Ui.button(this, "Open a code", false, v -> openCode(null)), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        p.addView(codes);

        // What to do with it.
        LinearLayout actions = new LinearLayout(this);
        actions.setPadding(0, Ui.dp(this, 8), 0, 0);
        actions.addView(Ui.button(this, "Wallpaper", true, v -> setLive()), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.2f));
        actions.addView(gap(6));
        actions.addView(Ui.button(this, "Still", false, v -> askStill()), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 0.8f));
        actions.addView(gap(6));
        actions.addView(Ui.button(this, "Image", false, v -> saveImage()), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 0.8f));
        actions.addView(gap(6));
        actions.addView(Ui.button(this, "GIF/WebP", false, v -> askAnimation()), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.1f));
        p.addView(actions);
        return p;
    }

    // ── Scenes: yours, then the presets ──────────────────────────────────

    private void refreshStrip() {
        strip.removeAllViews();
        tiles.clear();
        for (String n : Library.names(this)) strip.addView(tile(n, true));
        if (!tiles.isEmpty()) {
            View line = new View(this);
            line.setBackgroundColor(Color.argb(60, 255, 255, 255));
            LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(Ui.dp(this, 1), Ui.dp(this, 120));
            lp.rightMargin = Ui.dp(this, 10);
            strip.addView(line, lp);
        }
        for (String n : presets) strip.addView(tile(n, false));
        highlight();
        loadThumbs();
    }

    private View tile(String name, boolean mine) {
        LinearLayout item = new LinearLayout(this);
        item.setOrientation(LinearLayout.VERTICAL);
        item.setGravity(Gravity.CENTER_HORIZONTAL);
        int w = Ui.dp(this, 68);
        ImageView img = new ImageView(this);
        img.setScaleType(ImageView.ScaleType.CENTER_CROP);
        img.setClipToOutline(true);
        img.setPadding(Ui.dp(this, 2), Ui.dp(this, 2), Ui.dp(this, 2), Ui.dp(this, 2));
        item.addView(img, new LinearLayout.LayoutParams(w, Math.round(w * 16f / 9f)));
        TextView label = new TextView(this);
        label.setText(mine ? "★ " + name : name);
        label.setTextColor(Ui.TEXT);
        label.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11);
        label.setGravity(Gravity.CENTER_HORIZONTAL);
        label.setMaxLines(2);
        item.addView(label, new LinearLayout.LayoutParams(w, ViewGroup.LayoutParams.WRAP_CONTENT));
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        lp.rightMargin = Ui.dp(this, 10);
        item.setLayoutParams(lp);
        item.setOnClickListener(v -> {
            String s = mine ? Library.load(this, name) : Native.presetScene(name);
            if (s == null) return;
            look.name = name;
            look.mine = mine;
            look.scene = s;
            sceneChanged(true);
        });
        item.setOnLongClickListener(v -> { sceneMenu(name, mine); return true; });
        tiles.add(new Object[]{name, mine, img});
        return item;
    }

    private void highlight() {
        for (Object[] t : tiles) {
            boolean on = t[0].equals(look.name) && (Boolean) t[1] == look.mine;
            ((ImageView) t[2]).setBackground(Ui.round(on ? Ui.ACCENT : Color.argb(30, 255, 255, 255), Ui.dp(this, 10), 0, 0));
        }
    }

    private void sceneMenu(String name, boolean mine) {
        if (!mine) {
            new AlertDialog.Builder(this).setTitle(name).setItems(new String[]{"Save a copy as one of yours"}, (d, i) -> {
                String s = Native.presetScene(name);
                if (s != null) askSave(name + " (mine)", s);
            }).show();
            return;
        }
        new AlertDialog.Builder(this).setTitle(name).setItems(new String[]{"Rename", "Duplicate", "Export to a file", "Delete"}, (d, i) -> {
            String s = Library.load(this, name);
            switch (i) {
                case 0: askName("Rename", name, to -> {
                    if (Library.exists(this, to)) { toast("There is already a scene called " + to); return; }
                    if (!Library.rename(this, name, to)) { toast("Could not rename it"); return; }
                    String renamed = named(Library.load(this, to), to);
                    if (renamed != null) Library.save(this, to, renamed);
                    if (look.mine && look.name.equals(name)) { look.name = to; if (renamed != null) look.scene = renamed; sceneChanged(false); }
                    refreshStrip();
                }); break;
                case 1: if (s != null) askSave(name + " copy", s); break;
                case 2: if (s != null) exportScene(name, s); break;
                default:
                    new AlertDialog.Builder(this).setMessage("Delete " + name + "?").setPositiveButton("Delete", (dd, w) -> {
                        Library.delete(this, name);
                        if (look.mine && look.name.equals(name)) look.mine = false;
                        refreshStrip();
                        sceneChanged(false);
                    }).setNegativeButton("Cancel", null).show();
            }
        }).show();
    }

    interface Named { void name(String n); }

    private void askName(String title, String start, Named then) {
        EditText e = new EditText(this);
        e.setSingleLine(true);
        e.setText(start);
        e.setSelectAllOnFocus(true);
        FrameLayout box = new FrameLayout(this);
        int p = Ui.dp(this, 20);
        box.setPadding(p, p / 2, p, 0);
        box.addView(e);
        new AlertDialog.Builder(this).setTitle(title).setView(box).setPositiveButton("OK", (d, w) -> {
            String n = e.getText().toString().trim();
            if (n.isEmpty()) { toast("It needs a name"); return; }
            then.name(n);
        }).setNegativeButton("Cancel", null).show();
        e.requestFocus();
    }

    /** `scene` with its name set to `name`. */
    private static String named(String scene, String name) {
        if (scene == null) return null;
        try {
            JSONObject o = new JSONObject(scene);
            o.put("name", name);
            return o.toString();
        } catch (JSONException e) {
            return scene;
        }
    }

    /** Keep `scene` (the current one when null) as one of yours, asking for its name. */
    private void askSave(String suggested, String scene) {
        askName("Save scene as", suggested, n -> {
            Runnable save = () -> {
                String s = named(scene == null ? look.scene() : scene, n);
                if (!Library.save(this, n, s)) { toast("Could not save it"); return; }
                look.name = n;
                look.mine = true;
                look.scene = s;
                sceneChanged(true);
                refreshStrip();
                toast("Saved " + n);
            };
            boolean same = look.mine && look.name.equals(n) && scene == null;
            if (Library.exists(this, n) && !same) {
                new AlertDialog.Builder(this).setMessage("Replace your scene " + n + "?").setPositiveButton("Replace", (d, w) -> save.run()).setNegativeButton("Cancel", null).show();
            } else save.run();
        });
    }

    private void exportScene(String name, String scene) {
        exporting = scene;
        Intent i = new Intent(Intent.ACTION_CREATE_DOCUMENT);
        i.addCategory(Intent.CATEGORY_OPENABLE);
        i.setType("application/json");
        i.putExtra(Intent.EXTRA_TITLE, name + ".json");
        try { startActivityForResult(i, EXPORT_SCENE); } catch (ActivityNotFoundException e) { toast("No file picker on this phone"); }
    }

    // ── Surprise me, and share codes ─────────────────────────────────────

    private static final String[] PARTS = {"camera", "path", "setting", "sky", "lighting", "props", "atmosphere", "look"};
    private static final String[] PART_LABELS = {"Camera", "Path", "Walls and verge", "Sky", "Lights", "Props and wildlife", "Weather and season", "Look and lens"};
    /** Scenes to go back to after a surprise or an opened code. */
    private final java.util.ArrayDeque<String[]> history = new java.util.ArrayDeque<>();
    private TextView backButton;

    private String locks() { return getSharedPreferences("look", MODE_PRIVATE).getString("locks", "camera"); }

    /** Which parts "Surprise me" keeps as they are. */
    private void askLocks() {
        List<String> on = java.util.Arrays.asList(locks().split(","));
        boolean[] checked = new boolean[PARTS.length];
        for (int i = 0; i < PARTS.length; i++) checked[i] = on.contains(PARTS[i]);
        new AlertDialog.Builder(this).setTitle("Keep these as they are")
                .setMultiChoiceItems(PART_LABELS, checked, (d, i, v) -> checked[i] = v)
                .setPositiveButton("Surprise me", (d, w) -> { saveLocks(checked); surprise(); })
                .setNegativeButton("Done", (d, w) -> saveLocks(checked))
                .show();
    }

    private void saveLocks(boolean[] checked) {
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < PARTS.length; i++) if (checked[i]) b.append(b.length() > 0 ? "," : "").append(PARTS[i]);
        getSharedPreferences("look", MODE_PRIVATE).edit().putString("locks", b.toString()).apply();
    }

    /** A new scene: the locked parts of this one, the rest from presets picked at random. */
    private void surprise() {
        String from = look.scene(), keep = locks();
        int seed = (int) (Math.random() * 1_000_000);
        work.execute(() -> {
            String s = Native.remix(from, keep, seed);
            ui.post(() -> {
                if (s == null) { toast("Could not make one: " + Native.lastError()); return; }
                show(s, null);
            });
        });
    }

    /** Make `scene` the current one (remembering this one for Back). */
    private void show(String scene, String name) {
        history.push(new String[]{look.scene(), look.name, String.valueOf(look.mine)});
        while (history.size() > 30) history.removeLast();
        backButton.setAlpha(1f);
        look.scene = scene;
        try { look.name = name != null ? name : new JSONObject(scene).optString("name", "Surprise"); } catch (JSONException e) { look.name = "Surprise"; }
        look.mine = false;
        sceneChanged(true);
        refreshStrip();
    }

    private void back() {
        String[] h = history.poll();
        if (h == null) return;
        look.scene = h[0];
        look.name = h[1];
        look.mine = Boolean.parseBoolean(h[2]);
        backButton.setAlpha(history.isEmpty() ? 0.4f : 1f);
        sceneChanged(true);
        refreshStrip();
    }

    /** The scene as a share code: a line of text and its QR code, to copy or send. */
    private void shareCode() {
        String scene = look.scene();
        work.execute(() -> {
            String code = Native.shareCode(scene);
            byte[] qr = code == null ? null : Native.qr(code);
            ui.post(() -> {
                if (code == null) { toast("Could not make a code: " + Native.lastError()); return; }
                LinearLayout box = new LinearLayout(this);
                box.setOrientation(LinearLayout.VERTICAL);
                int pad = Ui.dp(this, 20);
                box.setPadding(pad, pad / 2, pad, 0);
                Bitmap q = qrBitmap(qr);
                if (q != null) {
                    ImageView iv = new ImageView(this);
                    iv.setImageBitmap(q);
                    iv.setAdjustViewBounds(true);
                    box.addView(iv, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
                }
                TextView t = new TextView(this);
                t.setText(code);
                t.setTextIsSelectable(true);
                t.setTypeface(Typeface.MONOSPACE);
                t.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11);
                t.setMaxLines(6);
                t.setPadding(0, pad / 2, 0, 0);
                box.addView(t);
                new AlertDialog.Builder(this).setTitle(look.name).setView(box)
                        .setPositiveButton("Send…", (d, w) -> {
                            Intent i = new Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, code);
                            try { startActivity(Intent.createChooser(i, "Share the scene")); } catch (ActivityNotFoundException e) { toast("Nothing here can send text"); }
                        })
                        .setNeutralButton("Copy", (d, w) -> {
                            android.content.ClipboardManager cm = (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
                            if (cm != null) { cm.setPrimaryClip(android.content.ClipData.newPlainText("PathForge scene", code)); toast("Copied"); }
                        })
                        .setNegativeButton("Close", null).show();
            });
        });
    }

    /** A QR code from Native.qr (its width, then a byte a module) as a bitmap with a white margin. */
    private static Bitmap qrBitmap(byte[] q) {
        if (q == null || q.length < 2) return null;
        int n = q[0] & 0xFF, scale = 8, side = (n + 8) * scale;
        if (q.length < 1 + n * n) return null;
        int[] px = new int[side * side];
        java.util.Arrays.fill(px, Color.WHITE);
        for (int my = 0; my < n; my++) for (int mx = 0; mx < n; mx++) {
            if (q[1 + my * n + mx] == 0) continue;
            for (int y = (my + 4) * scale; y < (my + 5) * scale; y++) java.util.Arrays.fill(px, y * side + (mx + 4) * scale, y * side + (mx + 5) * scale, Color.BLACK);
        }
        return Bitmap.createBitmap(px, side, side, Bitmap.Config.ARGB_8888);
    }

    /** Open a share code: pasted, or `given` (one sent to the app). */
    private void openCode(String given) {
        EditText e = new EditText(this);
        e.setHint("PF1.…");
        e.setTypeface(Typeface.MONOSPACE);
        String start = given;
        if (start == null) {
            android.content.ClipboardManager cm = (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
            CharSequence c = cm != null && cm.hasPrimaryClip() && cm.getPrimaryClip().getItemCount() > 0 ? cm.getPrimaryClip().getItemAt(0).coerceToText(this) : null;
            if (c != null && c.toString().trim().startsWith("PF1.")) start = c.toString().trim();
        }
        if (start != null) e.setText(start);
        int pad = Ui.dp(this, 20);
        FrameLayout f = new FrameLayout(this);
        f.setPadding(pad, pad / 2, pad, 0);
        f.addView(e);
        new AlertDialog.Builder(this).setTitle("Open a share code").setView(f)
                .setPositiveButton("Open", (d, w) -> {
                    String code = e.getText().toString();
                    work.execute(() -> {
                        String s = Native.unshare(code);
                        ui.post(() -> {
                            if (s == null) { toast(Native.lastError()); return; }
                            show(s, null);
                            toast("Opened " + look.name + ". Save scene keeps it.");
                        });
                    });
                })
                .setNegativeButton("Cancel", null).show();
    }

    // ── The editor ───────────────────────────────────────────────────────

    private void openEditor() {
        editor.load(look.scene());
        panel.setVisibility(View.GONE);
        showButton.setVisibility(View.GONE);
        editor.view.setVisibility(View.VISIBLE);
        preview.setFitTop(0.4f, topInset);
    }

    @Override
    public void changed(String scene) {
        look.scene = scene;
        ui.removeCallbacks(applyEdit);
        ui.postDelayed(applyEdit, 120);
    }

    @Override
    public int[] screen() { Point p = screenSize(); return new int[]{p.x, p.y}; }

    @Override
    public boolean canvasPreview() { return getSharedPreferences("look", MODE_PRIVATE).getBoolean("canvas_shape", true); }

    @Override
    public void setCanvasPreview(boolean on) {
        getSharedPreferences("look", MODE_PRIVATE).edit().putBoolean("canvas_shape", on).apply();
        preview.setCanvasShape(on);
        if (shapeChips != null) shapeChips.select(on ? 0 : 1);
    }

    private Ui.Chips shapeChips;

    /** The scene's canvas, pixels (480 x 854 when it has none). */
    private Point canvas() {
        try {
            org.json.JSONObject c = new org.json.JSONObject(look.scene()).optJSONObject("canvas");
            int w = c == null ? 480 : c.optInt("width", 480), h = c == null ? 854 : c.optInt("height", 854);
            if (Editor.canvasProblem(w, h) == null) return new Point(w, h);
        } catch (JSONException ignored) {
        }
        return new Point(480, 854);
    }

    @Override
    public void peek(String scene) {
        ui.removeCallbacks(applyEdit);
        preview.setScene(scene != null ? scene : look.scene());
    }

    @Override
    public void done() {
        ui.removeCallbacks(applyEdit);
        applyEdit.run();
        editor.view.setVisibility(View.GONE);
        preview.setFitTop(0, 0);
        showPanel(true);
        sceneChanged(true);
        if (look.mine) toast("Changed. Save scene keeps the changes in " + look.name);
    }

    private void showSpeed() { speedLabel.setText(String.format(java.util.Locale.US, "Walking speed  %.1f m/s", look.speed())); }

    /** The scene or a change to it: save it, show it, and (when `controls`) set the controls to it. */
    private void sceneChanged(boolean controls) {
        look.save(this);
        title.setText(look.mine ? "★ " + look.name : look.name);
        highlight();
        if (controls) {
            skyChips.select(look.sky());
            timeChips.select(look.time());
            seasonChips.select(look.season());
            cloudChips.select(look.mixed() ? 1 : 0);
            speedBar.setProgress((int) Math.round((look.speed() - 0.5) / 0.1));
            showSpeed();
        }
        preview.setScene(look.scene());
    }

    private void showPanel(boolean show) {
        panel.setVisibility(show ? View.VISIBLE : View.GONE);
        showButton.setVisibility(show ? View.GONE : View.VISIBLE);
    }

    @Override
    public void onBackPressed() {
        if (editor.view.getVisibility() == View.VISIBLE) done();
        else if (panel.getVisibility() != View.VISIBLE) showPanel(true);
        else super.onBackPressed();
    }

    @Override protected void onResume() { super.onResume(); preview.resume(); }
    @Override protected void onPause() { preview.pause(); look.save(this); super.onPause(); }

    @Override
    protected void onDestroy() {
        work.shutdownNow();
        thumbs.shutdownNow();
        super.onDestroy();
    }

    // ── Thumbnails ───────────────────────────────────────────────────────

    /** A small first frame of every scene in the strip, made once and kept (presets per version
     *  of the app, yours per save). */
    private void loadThumbs() {
        long version;
        try { version = getPackageManager().getPackageInfo(getPackageName(), 0).getLongVersionCode(); } catch (Exception e) { version = 0; }
        File dir = new File(getCacheDir(), "thumbs");
        //noinspection ResultOfMethodCallIgnored
        dir.mkdirs();
        final long v = version;
        final List<Object[]> todo = new ArrayList<>(tiles);
        thumbs.execute(() -> {
            int w = 90, h = 160;
            ByteBuffer buf = ByteBuffer.allocateDirect(w * h * 4);
            for (Object[] t : todo) {
                String name = (String) t[0];
                boolean mine = (Boolean) t[1];
                String key = mine ? "mine_" + Integer.toHexString(name.hashCode()) + "_" + Library.modified(this, name) : "v" + v + "_" + Integer.toHexString(name.hashCode());
                File f = new File(dir, key + ".png");
                Bitmap b = f.exists() ? BitmapFactory.decodeFile(f.getPath()) : null;
                if (b == null) {
                    String json = mine ? Library.load(this, name) : Native.presetScene(name);
                    if (json == null) continue;
                    // Frames this small are quicker on the CPU than a trip to the GPU and back.
                    long hd = Native.open(json, "", false);
                    if (hd == 0) continue;
                    try {
                        if (!Native.render(hd, 0f, 0f, w, h, buf)) continue;
                    } finally {
                        Native.close(hd);
                    }
                    buf.rewind();
                    b = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888);
                    b.copyPixelsFromBuffer(buf);
                    buf.rewind();
                    try (FileOutputStream out = new FileOutputStream(f)) { b.compress(Bitmap.CompressFormat.PNG, 100, out); } catch (IOException ignored) { }
                }
                final Bitmap fb = b;
                ui.post(() -> ((ImageView) t[2]).setImageBitmap(fb));
            }
        });
    }

    // ── Wallpaper ────────────────────────────────────────────────────────

    /** The whole screen's size, in portrait. */
    private Point screenSize() {
        int w, h;
        if (Build.VERSION.SDK_INT >= 30) {
            Rect b = getWindowManager().getMaximumWindowMetrics().getBounds();
            w = b.width();
            h = b.height();
        } else {
            DisplayMetrics dm = new DisplayMetrics();
            //noinspection deprecation
            getWindowManager().getDefaultDisplay().getRealMetrics(dm);
            w = dm.widthPixels;
            h = dm.heightPixels;
        }
        return new Point(Math.min(w, h), Math.max(w, h));
    }

    private boolean oursIsLive() {
        WallpaperInfo info = WallpaperManager.getInstance(this).getWallpaperInfo();
        return info != null && info.getPackageName().equals(getPackageName());
    }

    /** A dialog with a progress bar; `cancel` is set when Cancel is pressed. */
    private final class Progress {
        final AlertDialog dialog;
        final ProgressBar bar;
        final TextView text;
        final AtomicBoolean cancel = new AtomicBoolean();

        Progress(String message, boolean cancellable) {
            LinearLayout box = new LinearLayout(MainActivity.this);
            box.setOrientation(LinearLayout.VERTICAL);
            int p = Ui.dp(MainActivity.this, 24);
            box.setPadding(p, p, p, p / 2);
            text = new TextView(MainActivity.this);
            text.setText(message);
            text.setTextSize(TypedValue.COMPLEX_UNIT_SP, 16);
            box.addView(text);
            bar = new ProgressBar(MainActivity.this, null, android.R.attr.progressBarStyleHorizontal);
            bar.setIndeterminate(true);
            bar.setPadding(0, p / 2, 0, 0);
            box.addView(bar);
            AlertDialog.Builder b = new AlertDialog.Builder(MainActivity.this).setView(box).setCancelable(false);
            if (cancellable) b.setNegativeButton("Cancel", (d, w) -> cancel.set(true));
            dialog = b.show();
        }

        void at(int done, int total) {
            ui.post(() -> {
                bar.setIndeterminate(false);
                bar.setMax(total);
                bar.setProgress(done);
            });
        }

        void close() { ui.post(dialog::dismiss); }
    }

    /** Bake the loop for the wallpaper, then make it the wallpaper (or update it if it already is). */
    private void setLive() {
        final String scene = look.scene();
        final int quality = look.quality;
        final Point size = screenSize();
        final Progress pr = new Progress("Drawing your wallpaper's loop…", true);
        preview.pause();
        work.execute(() -> {
            try {
                Bake.bake(this, scene, quality, size.x, size.y, new Bake.Progress() {
                    @Override public void at(int done, int total) { pr.at(done, total); }
                    @Override public boolean cancelled() { return pr.cancel.get(); }
                });
                Bake.bump(this);
                pr.close();
                ui.post(() -> {
                    preview.resume();
                    if (oursIsLive()) toast("Wallpaper updated");
                    else openPicker();
                });
            } catch (IOException | RuntimeException e) {
                pr.close();
                ui.post(() -> {
                    preview.resume();
                    if (!pr.cancel.get()) toast("Could not make the wallpaper: " + e.getMessage());
                });
            }
        });
    }

    private void openPicker() {
        Intent i = new Intent(WallpaperManager.ACTION_CHANGE_LIVE_WALLPAPER);
        i.putExtra(WallpaperManager.EXTRA_LIVE_WALLPAPER_COMPONENT, new ComponentName(this, PathWallpaperService.class));
        try {
            startActivity(i);
        } catch (ActivityNotFoundException e) {
            try {
                startActivity(new Intent(WallpaperManager.ACTION_LIVE_WALLPAPER_CHOOSER));
                toast("Choose PathForge in the list");
            } catch (ActivityNotFoundException e2) {
                toast("This phone has no live wallpaper picker; use Still instead");
            }
        }
    }

    private void askStill() {
        String[] where = {"Home screen", "Lock screen", "Home and lock screens"};
        int[] flags = {WallpaperManager.FLAG_SYSTEM, WallpaperManager.FLAG_LOCK, WallpaperManager.FLAG_SYSTEM | WallpaperManager.FLAG_LOCK};
        new AlertDialog.Builder(this).setTitle("Set this moment as a still wallpaper")
                .setItems(where, (d, i) -> still(flags[i])).show();
    }

    /** This moment of the walk at the screen's full size, on a worker thread, then `done` on the UI thread. */
    private void renderMoment(String message, java.util.function.Consumer<Bitmap> done) { renderMoment(message, screenSize(), done); }

    /** `renderMoment` at `size` pixels. */
    private void renderMoment(String message, Point size, java.util.function.Consumer<Bitmap> done) {
        final String scene = look.scene();
        final float d = preview.distance(), t = preview.time();
        final Progress pr = new Progress(message, false);
        work.execute(() -> {
            Bitmap bmp = null;
            String err = null;
            long h = Gpu.open(this, scene);
            if (h == 0) err = Native.lastError();
            else {
                try {
                    ByteBuffer buf = ByteBuffer.allocateDirect(size.x * size.y * 4);
                    if (Native.render(h, d, t, size.x, size.y, buf)) {
                        buf.rewind();
                        bmp = Bitmap.createBitmap(size.x, size.y, Bitmap.Config.ARGB_8888);
                        bmp.copyPixelsFromBuffer(buf);
                    } else err = Native.lastError();
                } catch (OutOfMemoryError e) {
                    err = "not enough memory";
                } finally {
                    Native.close(h);
                }
            }
            pr.close();
            final Bitmap b = bmp;
            final String e = err;
            ui.post(() -> { if (b != null) done.accept(b); else toast("Could not draw it: " + e); });
        });
    }

    private void still(int flags) {
        renderMoment("Drawing it at full size…", b -> work.execute(() -> {
            try {
                WallpaperManager.getInstance(this).setBitmap(b, null, true, flags);
                ui.post(() -> toast("Wallpaper set"));
            } catch (IOException | RuntimeException e) {
                ui.post(() -> toast("Could not set the wallpaper: " + e.getMessage()));
            }
        }));
    }

    private void saveImage() {
        // At the canvas's size and shape, or the screen's when the preview fills the screen.
        Point size = canvasPreview() ? canvas() : screenSize();
        renderMoment("Drawing it at " + size.x + " × " + size.y + "…", size, b -> work.execute(() -> {
            ContentResolver r = getContentResolver();
            ContentValues v = new ContentValues();
            v.put(MediaStore.Images.Media.DISPLAY_NAME, "PathForge " + System.currentTimeMillis() + ".png");
            v.put(MediaStore.Images.Media.MIME_TYPE, "image/png");
            v.put(MediaStore.Images.Media.RELATIVE_PATH, Environment.DIRECTORY_PICTURES + "/PathForge");
            v.put(MediaStore.Images.Media.IS_PENDING, 1);
            Uri uri = r.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, v);
            if (uri == null) { ui.post(() -> toast("Could not save the picture")); return; }
            try (OutputStream out = r.openOutputStream(uri)) {
                if (out == null) throw new IOException("no stream");
                b.compress(Bitmap.CompressFormat.PNG, 100, out);
                v.clear();
                v.put(MediaStore.Images.Media.IS_PENDING, 0);
                r.update(uri, v, null, null);
                ui.post(() -> toast("Saved to Pictures/PathForge"));
            } catch (IOException e) {
                r.delete(uri, null, null);
                ui.post(() -> toast("Could not save the picture: " + e.getMessage()));
            }
        }));
    }

    // ── The loop as an animation ─────────────────────────────────────────

    /** Choose the format, size and frame rate, then write the whole loop as a seamless animation. */
    private void askAnimation() {
        SharedPreferences p = getSharedPreferences("look", MODE_PRIVATE);
        final int[] pick = {p.getInt("anim_format", 0), p.getInt("anim_size", 1), p.getInt("anim_fps", 1)};
        LinearLayout box = new LinearLayout(this);
        box.setOrientation(LinearLayout.VERTICAL);
        int pad = Ui.dp(this, 20);
        box.setPadding(pad, pad / 2, pad, 0);
        box.addView(Ui.label(this, "Format"));
        box.addView(new Ui.Chips(this, new String[]{"WebP (full colour, small)", "GIF (plays anywhere)"}, pick[0], i -> pick[0] = i).view);
        box.addView(Ui.label(this, "Size"));
        Point cv = canvas();
        boolean byCanvas = canvasPreview();
        box.addView(new Ui.Chips(this, new String[]{"Small", "Medium", "Large", byCanvas ? "Canvas (" + cv.x + "×" + cv.y + ")" : "Full screen"}, pick[1], i -> pick[1] = i).view);
        box.addView(Ui.label(this, "Frames a second"));
        box.addView(new Ui.Chips(this, new String[]{"12", "20", "30"}, pick[2], i -> pick[2] = i).view);
        TextView note = new TextView(this);
        note.setText("The whole loop, joining end to start, so it plays forever without a seam. Larger and smoother takes longer and makes a bigger file (GIF most of all).");
        note.setTextColor(Ui.DIM);
        note.setPadding(0, pad / 2, 0, 0);
        box.addView(note);
        new AlertDialog.Builder(this).setTitle("Save the loop as an animation").setView(box)
                .setPositiveButton("Make it", (d, w) -> {
                    p.edit().putInt("anim_format", pick[0]).putInt("anim_size", pick[1]).putInt("anim_fps", pick[2]).apply();
                    exportAnimation(pick[0] == 0 ? "webp" : "gif", pick[1], new int[]{12, 20, 30}[pick[2]]);
                }).setNegativeButton("Cancel", null).show();
    }

    private void exportAnimation(String format, int size, int fps) {
        // The canvas's shape (or the screen's): Small, Medium and Large on its short side, or its
        // own size.
        Point shape = canvasPreview() ? canvas() : screenSize();
        int w, h;
        if (size == 3) { w = shape.x; h = shape.y; }
        else {
            int s = new int[]{360, 540, 720}[size];
            float k = s / (float) Math.min(shape.x, shape.y);
            w = Math.round(shape.x * k);
            h = Math.round(shape.y * k);
        }
        w &= ~1;
        h &= ~1;
        final String scene = look.scene();
        final String name = look.name.replaceAll("[\\/:*?\"<>|]", "_");
        final File dir = new File(getCacheDir(), "export");
        //noinspection ResultOfMethodCallIgnored
        dir.mkdirs();
        final Progress pr = new Progress("Drawing the loop (" + w + "×" + h + ", " + fps + " fps)…", true);
        final boolean gpu = Gpu.wanted(this);
        preview.pause();
        // How far it has got, while it runs.
        final Runnable[] poll = new Runnable[1];
        poll[0] = () -> {
            int[] at = Native.exportProgress();
            if (at != null && at[1] > 0) pr.at(at[0], at[1]);
            if (pr.cancel.get()) Native.exportCancel();
            ui.postDelayed(poll[0], 250);
        };
        ui.postDelayed(poll[0], 250);
        final int fw = w, fh = h;
        work.execute(() -> {
            String path = Native.exportAnimation(scene, dir.getPath(), name, format, fw, fh, fps, format.equals("gif") ? 0.3f : 88f, gpu);
            ui.removeCallbacks(poll[0]);
            pr.close();
            if (path == null) {
                String why = Native.lastError();
                ui.post(() -> { preview.resume(); if (!pr.cancel.get()) toast("Could not make it: " + why); });
                return;
            }
            Uri uri = savePicture(new File(path), format.equals("gif") ? "image/gif" : "image/webp");
            //noinspection ResultOfMethodCallIgnored
            new File(path).delete();
            ui.post(() -> {
                preview.resume();
                if (uri == null) { toast("Could not save it to Pictures"); return; }
                new AlertDialog.Builder(this).setTitle("Saved to Pictures/PathForge")
                        .setMessage("It loops seamlessly. Share it now?")
                        .setPositiveButton("Share", (d, wh) -> {
                            Intent send = new Intent(Intent.ACTION_SEND);
                            send.setType(format.equals("gif") ? "image/gif" : "image/webp");
                            send.putExtra(Intent.EXTRA_STREAM, uri);
                            send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
                            startActivity(Intent.createChooser(send, "Share the loop"));
                        }).setNegativeButton("Done", null).show();
            });
        });
    }

    /** Copy a file into Pictures/PathForge; its content URI, or null. */
    private Uri savePicture(File f, String mime) {
        ContentResolver r = getContentResolver();
        ContentValues v = new ContentValues();
        v.put(MediaStore.Images.Media.DISPLAY_NAME, f.getName().replaceAll("\\.[a-z]+$", "") + " " + System.currentTimeMillis() + (mime.endsWith("gif") ? ".gif" : ".webp"));
        v.put(MediaStore.Images.Media.MIME_TYPE, mime);
        v.put(MediaStore.Images.Media.RELATIVE_PATH, Environment.DIRECTORY_PICTURES + "/PathForge");
        v.put(MediaStore.Images.Media.IS_PENDING, 1);
        Uri uri = r.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, v);
        if (uri == null) return null;
        try (OutputStream out = r.openOutputStream(uri); java.io.FileInputStream in = new java.io.FileInputStream(f)) {
            if (out == null) throw new IOException("no stream");
            byte[] b = new byte[1 << 16];
            for (int n; (n = in.read(b)) > 0; ) out.write(b, 0, n);
        } catch (IOException e) {
            r.delete(uri, null, null);
            return null;
        }
        v.clear();
        v.put(MediaStore.Images.Media.IS_PENDING, 0);
        r.update(uri, v, null, null);
        return uri;
    }

    // ── Scenes made on the desktop ───────────────────────────────────────

    private void pickScene() {
        Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        i.addCategory(Intent.CATEGORY_OPENABLE);
        i.setType("*/*");
        i.putExtra(Intent.EXTRA_MIME_TYPES, new String[]{"application/json", "text/plain", "application/octet-stream"});
        try {
            startActivityForResult(i, PICK_SCENE);
        } catch (ActivityNotFoundException e) {
            toast("No file picker on this phone");
        }
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (result != RESULT_OK || data == null || data.getData() == null) return;
        Uri uri = data.getData();
        if (request == EXPORT_SCENE) {
            final String scene = exporting;
            exporting = null;
            if (scene == null) return;
            work.execute(() -> {
                try (OutputStream out = getContentResolver().openOutputStream(uri)) {
                    if (out == null) throw new IOException("no stream");
                    out.write(scene.getBytes(java.nio.charset.StandardCharsets.UTF_8));
                    ui.post(() -> toast("Exported. Open it in PathForge on the desktop."));
                } catch (IOException e) {
                    ui.post(() -> toast("Could not export it: " + e.getMessage()));
                }
            });
            return;
        }
        if (request != PICK_SCENE) return;
        work.execute(() -> {
            String json;
            try { json = Look.read(getContentResolver().openInputStream(uri)); } catch (IOException | RuntimeException e) { json = null; }
            String scene = json == null ? null : Native.normalize(json);
            if (scene == null) {
                String why = json == null ? "could not read the file" : Native.lastError();
                ui.post(() -> toast("Not a PathForge scene: " + why));
                return;
            }
            String name;
            try { name = new JSONObject(scene).optString("name", ""); } catch (JSONException e) { name = ""; }
            if (name.isEmpty()) name = "Imported scene";
            final String n = name;
            ui.post(() -> {
                look.name = n;
                look.mine = false;
                look.scene = scene;
                sceneChanged(true);
                toast("Imported " + n + ". Save scene keeps it with yours. (Scenes that use image files show without them.)");
            });
        });
    }

    private void toast(String s) { Toast.makeText(this, s, Toast.LENGTH_LONG).show(); }
}

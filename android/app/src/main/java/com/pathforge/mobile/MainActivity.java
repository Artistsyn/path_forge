package com.pathforge.mobile;

import android.app.Activity;
import android.app.AlertDialog;
import android.app.WallpaperInfo;
import android.app.WallpaperManager;
import android.content.ActivityNotFoundException;
import android.content.ComponentName;
import android.content.ContentResolver;
import android.content.ContentValues;
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
public final class MainActivity extends Activity {
    private static final int PICK_SCENE = 1;

    private Look look;
    private PreviewView preview;
    private LinearLayout panel;
    private TextView title;
    private TextView speedLabel;
    private TextView status;
    /** The GPU's name once it has started, "" if there is none, null until then. */
    private volatile String gpuName;
    private TextView showButton;
    private Ui.Chips skyChips, cloudChips;
    private final List<View> presetItems = new ArrayList<>();
    private String[] presets = new String[0];
    private final Handler ui = new Handler(Looper.getMainLooper());
    private final ExecutorService work = Executors.newSingleThreadExecutor();
    private final ExecutorService thumbs = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "pathforge-thumbs");
        t.setPriority(Thread.MIN_PRIORITY);
        return t;
    });

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
        preview.setOnClickListener(v -> showPanel(panel.getVisibility() != View.VISIBLE));
        root.addView(preview, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));

        panel = buildPanel();
        FrameLayout.LayoutParams plp = new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM);
        root.addView(panel, plp);

        showButton = Ui.button(this, "Edit", false, v -> showPanel(true));
        showButton.setBackground(Ui.round(Ui.PANEL, Ui.dp(this, 22), Color.argb(110, 255, 255, 255), Ui.dp(this, 1)));
        showButton.setVisibility(View.GONE);
        FrameLayout.LayoutParams slp = new FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM | Gravity.END);
        root.addView(showButton, slp);

        root.setOnApplyWindowInsetsListener((v, in) -> {
            int bottom = Build.VERSION.SDK_INT >= 30 ? in.getInsets(WindowInsets.Type.systemBars()).bottom : in.getSystemWindowInsetBottom();
            int p = Ui.dp(this, 16);
            panel.setPadding(p, p, p, p + bottom);
            FrameLayout.LayoutParams lp = (FrameLayout.LayoutParams) showButton.getLayoutParams();
            lp.setMargins(p, p, p, p + bottom);
            showButton.setLayoutParams(lp);
            return in;
        });
        setContentView(root);
        if (Gpu.crashedLastTime(this)) toast("The GPU stopped PathForge last time, so it draws on the CPU now. You can switch back under Draw on.");
        preview.setStats((engine, w, h, fps) -> ui.post(() -> status.setText(String.format(java.util.Locale.US,
                "%s  ·  %d×%d  ·  %.0f fps", engine.equals("gpu") ? "GPU" + (gpuName == null || gpuName.isEmpty() ? "" : " " + gpuName) : "CPU", w, h, fps))));
        if (Gpu.wanted(this)) work.execute(() -> gpuName = Native.gpuInit());
        sceneChanged();
        loadThumbs();
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
        View gap = new View(this);
        head.addView(gap, new LinearLayout.LayoutParams(Ui.dp(this, 8), 1));
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
        scroll.setLayoutParams(new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        scroll.post(() -> {
            int max = (int) (dm.heightPixels * 0.42f);
            if (scroll.getHeight() > max) scroll.setLayoutParams(new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, max));
        });

        body.addView(Ui.label(this, "Scenes"));
        HorizontalScrollView strip = new HorizontalScrollView(this);
        strip.setHorizontalScrollBarEnabled(false);
        LinearLayout row = new LinearLayout(this);
        strip.addView(row);
        for (int i = 0; i < presets.length; i++) row.addView(presetItem(i));
        body.addView(strip);

        body.addView(Ui.label(this, "Sky"));
        skyChips = new Ui.Chips(this, Look.SKIES, look.sky, i -> { look.sky = i; sceneChanged(); });
        body.addView(skyChips.view);

        body.addView(Ui.label(this, "Clouds"));
        LinearLayout clouds = new LinearLayout(this);
        clouds.setGravity(Gravity.CENTER_VERTICAL);
        cloudChips = new Ui.Chips(this, Look.CLOUDS, look.mixClouds ? 1 : 0, i -> { look.mixClouds = i == 1; sceneChanged(); });
        clouds.addView(cloudChips.view);
        clouds.addView(Ui.button(this, "Shuffle", false, v -> {
            look.mixClouds = true;
            look.cloudSeed = (int) (Math.random() * 9999);
            cloudChips.select(1);
            sceneChanged();
        }));
        body.addView(clouds);

        speedLabel = Ui.label(this, "");
        body.addView(speedLabel);
        SeekBar speed = new SeekBar(this);
        speed.setMax(35);
        speed.setProgress(Math.round((look.speed - 0.25f) / 0.05f));
        speed.getProgressDrawable().setTint(Ui.ACCENT);
        speed.getThumb().setTint(Ui.ACCENT);
        speed.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            @Override public void onProgressChanged(SeekBar s, int v, boolean user) {
                look.speed = 0.25f + v * 0.05f;
                showSpeed();
            }
            @Override public void onStartTrackingTouch(SeekBar s) { }
            @Override public void onStopTrackingTouch(SeekBar s) { sceneChanged(); }
        });
        body.addView(speed, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, Ui.dp(this, 40)));
        showSpeed();

        body.addView(Ui.label(this, "Live wallpaper"));
        body.addView(new Ui.Chips(this, Look.QUALITIES, look.quality, i -> { look.quality = i; look.save(this); }).view);

        body.addView(Ui.label(this, "Draw on"));
        body.addView(new Ui.Chips(this, new String[]{"GPU (fast)", "CPU"}, Gpu.wanted(this) ? 0 : 1, i -> {
            Gpu.setWanted(this, i == 0);
            preview.reopen();
        }).view);

        // What to do with it.
        LinearLayout actions = new LinearLayout(this);
        actions.setPadding(0, Ui.dp(this, 16), 0, 0);
        actions.addView(Ui.button(this, "Set as wallpaper", true, v -> setLive()), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.4f));
        View g1 = new View(this);
        actions.addView(g1, new LinearLayout.LayoutParams(Ui.dp(this, 8), 1));
        actions.addView(Ui.button(this, "Still", false, v -> askStill()), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 0.8f));
        View g2 = new View(this);
        actions.addView(g2, new LinearLayout.LayoutParams(Ui.dp(this, 8), 1));
        actions.addView(Ui.button(this, "Save", false, v -> saveImage()), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 0.8f));
        p.addView(actions);
        return p;
    }

    private View presetItem(int i) {
        LinearLayout item = new LinearLayout(this);
        item.setOrientation(LinearLayout.VERTICAL);
        item.setGravity(Gravity.CENTER_HORIZONTAL);
        int w = Ui.dp(this, 68);
        ImageView img = new ImageView(this);
        img.setScaleType(ImageView.ScaleType.CENTER_CROP);
        img.setClipToOutline(true);
        img.setPadding(Ui.dp(this, 2), Ui.dp(this, 2), Ui.dp(this, 2), Ui.dp(this, 2));
        item.addView(img, new LinearLayout.LayoutParams(w, Math.round(w * 16f / 9f)));
        TextView name = new TextView(this);
        name.setText(presets[i]);
        name.setTextColor(Ui.TEXT);
        name.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11);
        name.setGravity(Gravity.CENTER_HORIZONTAL);
        name.setMaxLines(2);
        item.addView(name, new LinearLayout.LayoutParams(w, ViewGroup.LayoutParams.WRAP_CONTENT));
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        lp.rightMargin = Ui.dp(this, 10);
        item.setLayoutParams(lp);
        item.setTag(img);
        item.setOnClickListener(v -> {
            String s = Native.presetScene(presets[i]);
            if (s == null) return;
            look.name = presets[i];
            look.base = s;
            sceneChanged();
        });
        presetItems.add(item);
        return item;
    }

    private void showSpeed() { speedLabel.setText(String.format(java.util.Locale.US, "Walking speed  %.2f×", look.speed)); }

    /** The scene or a change to it: save it, show it. */
    private void sceneChanged() {
        look.save(this);
        title.setText(look.name);
        for (int i = 0; i < presetItems.size(); i++) {
            ImageView img = (ImageView) presetItems.get(i).getTag();
            boolean on = presets[i].equals(look.name);
            img.setBackground(Ui.round(on ? Ui.ACCENT : Color.argb(30, 255, 255, 255), Ui.dp(this, 10), 0, 0));
        }
        preview.setScene(look.scene());
    }

    private void showPanel(boolean show) {
        panel.setVisibility(show ? View.VISIBLE : View.GONE);
        showButton.setVisibility(show ? View.GONE : View.VISIBLE);
    }

    @Override
    public void onBackPressed() {
        if (panel.getVisibility() != View.VISIBLE) showPanel(true); else super.onBackPressed();
    }

    @Override protected void onResume() { super.onResume(); preview.resume(); }
    @Override protected void onPause() { preview.pause(); super.onPause(); }

    @Override
    protected void onDestroy() {
        work.shutdownNow();
        thumbs.shutdownNow();
        super.onDestroy();
    }

    // ── Thumbnails ───────────────────────────────────────────────────────

    /** A small first frame of every preset, made once per version of the app and kept. */
    private void loadThumbs() {
        long version;
        try { version = getPackageManager().getPackageInfo(getPackageName(), 0).getLongVersionCode(); } catch (Exception e) { version = 0; }
        File dir = new File(getCacheDir(), "thumbs");
        //noinspection ResultOfMethodCallIgnored
        dir.mkdirs();
        final long v = version;
        thumbs.execute(() -> {
            int w = 90, h = 160;
            ByteBuffer buf = ByteBuffer.allocateDirect(w * h * 4);
            for (int i = 0; i < presets.length; i++) {
                File f = new File(dir, "v" + v + "_" + i + ".png");
                Bitmap b = f.exists() ? BitmapFactory.decodeFile(f.getPath()) : null;
                if (b == null) {
                    // Frames this small are quicker on the CPU than a trip to the GPU and back.
                    long hd = Native.open(Native.presetScene(presets[i]), "", false);
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
                final int k = i;
                ui.post(() -> ((ImageView) presetItems.get(k).getTag()).setImageBitmap(fb));
            }
        });
    }

    // ── Wallpaper ────────────────────────────────────────────────────────

    /** The whole screen's size, in portrait. */
    private Point screen() {
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
        final Point size = screen();
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
    private void renderMoment(String message, java.util.function.Consumer<Bitmap> done) {
        final String scene = look.scene();
        final float d = preview.distance(), t = preview.time();
        final Point size = screen();
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
        renderMoment("Drawing it at full size…", b -> work.execute(() -> {
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
        if (request != PICK_SCENE || result != RESULT_OK || data == null || data.getData() == null) return;
        Uri uri = data.getData();
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
                look.base = scene;
                sceneChanged();
                toast("Imported " + n + " (scenes that use image files show without them)");
            });
        });
    }

    private void toast(String s) { Toast.makeText(this, s, Toast.LENGTH_LONG).show(); }
}

package com.pathforge.mobile;

import android.app.Activity;
import android.app.AlertDialog;
import android.graphics.Color;
import android.graphics.Typeface;
import android.text.InputType;
import android.text.SpannableStringBuilder;
import android.text.Spanned;
import android.text.style.ForegroundColorSpan;
import android.text.style.RelativeSizeSpan;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.inputmethod.EditorInfo;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.SeekBar;
import android.widget.Switch;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;

/** Every setting of the scene, built from the renderer's description of them (Native.form, the
 *  same schema and slider ranges as the desktop studio's inspector): switches, sliders, choices,
 *  colours and lists, part by part. Long-press a setting for what it does. Each change goes to
 *  `Host.changed`; Undo steps back through them. */
final class Editor {
    interface Host {
        /** The scene, changed. */
        void changed(String scene);
        void done();
        /** Show `scene` in the preview for a moment without changing anything (null: the scene
         *  being edited again). */
        void peek(String scene);
        /** The phone's screen in pixels, portrait (for the canvas's "This screen"). */
        int[] screen();
        /** Whether the preview shows the canvas's shape (or fills the screen), and setting it. */
        boolean canvasPreview();
        void setCanvasPreview(boolean on);
    }

    private final Activity a;
    private final Host host;
    private final List<JSONObject> parts = new ArrayList<>();
    private JSONObject scene = new JSONObject();
    private final ArrayDeque<String> undo = new ArrayDeque<>();
    private final ArrayDeque<String> redo = new ArrayDeque<>();
    private static final int UNDO_MAX = 100;
    /** The scene as it was when editing began, for Before. */
    private String original = "{}";
    /** The editor: a header, the parts as tabs, and the current part's settings. */
    final LinearLayout view;
    private final LinearLayout tabs;
    private final ScrollView scroll;
    private final LinearLayout body;
    private TextView undoButton, redoButton;
    private int part;
    private boolean advanced;
    /** Groups and list items that are open, by path. */
    private final Set<String> open = new HashSet<>();

    Editor(Activity a, String form, Host host) {
        this.a = a;
        this.host = host;
        try {
            JSONArray ps = new JSONObject(form).getJSONArray("parts");
            for (int i = 0; i < ps.length(); i++) parts.add(ps.getJSONObject(i));
        } catch (JSONException ignored) {
        }
        view = new LinearLayout(a);
        view.setOrientation(LinearLayout.VERTICAL);
        float r = dp(24);
        android.graphics.drawable.GradientDrawable bg = new android.graphics.drawable.GradientDrawable();
        bg.setColor(Ui.PANEL);
        bg.setCornerRadii(new float[]{r, r, r, r, 0, 0, 0, 0});
        view.setBackground(bg);
        view.setClickable(true);

        LinearLayout head = new LinearLayout(a);
        head.setGravity(Gravity.CENTER_VERTICAL);
        TextView title = new TextView(a);
        title.setText("Edit");
        title.setTextColor(Ui.TEXT);
        title.setTextSize(TypedValue.COMPLEX_UNIT_SP, 20);
        title.setTypeface(Typeface.DEFAULT_BOLD);
        head.addView(title, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        undoButton = Ui.button(a, "Undo", false, v -> undo());
        head.addView(undoButton);
        head.addView(gap());
        redoButton = Ui.button(a, "Redo", false, v -> redo());
        head.addView(redoButton);
        head.addView(gap());
        // Hold to see the scene as it was before these edits; let go to see it as it is.
        TextView before = Ui.button(a, "Before", false, v -> { });
        before.setOnTouchListener((v, e) -> {
            switch (e.getActionMasked()) {
                case android.view.MotionEvent.ACTION_DOWN: v.setPressed(true); host.peek(original); return true;
                case android.view.MotionEvent.ACTION_UP:
                case android.view.MotionEvent.ACTION_CANCEL: v.setPressed(false); host.peek(null); v.performClick(); return true;
                default: return true;
            }
        });
        head.addView(before);
        head.addView(gap());
        head.addView(Ui.button(a, "Done", true, v -> host.done()));
        view.addView(head);

        TextView hint = new TextView(a);
        hint.setText("Long-press any setting for what it does. Hold Before to compare.");
        hint.setTextColor(Ui.DIM);
        hint.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
        hint.setPadding(0, dp(4), 0, dp(6));
        view.addView(hint);

        android.widget.HorizontalScrollView ts = new android.widget.HorizontalScrollView(a);
        ts.setHorizontalScrollBarEnabled(false);
        tabs = new LinearLayout(a);
        ts.addView(tabs);
        view.addView(ts);

        scroll = new ScrollView(a);
        body = new LinearLayout(a);
        body.setOrientation(LinearLayout.VERTICAL);
        body.setPadding(0, dp(8), 0, dp(24));
        scroll.addView(body);
        view.addView(scroll, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));
    }

    /** Start editing `json`. */
    void load(String json) {
        try { scene = new JSONObject(json); } catch (JSONException e) { scene = new JSONObject(); }
        original = scene.toString();
        staged = null;
        undo.clear();
        redo.clear();
        rebuild();
    }

    // ── Values by path ───────────────────────────────────────────────────

    private Object get(List<Object> path) {
        Object at = scene;
        for (Object k : path) {
            if (at instanceof JSONObject && k instanceof String) at = ((JSONObject) at).opt((String) k);
            else if (at instanceof JSONArray && k instanceof Integer) at = ((JSONArray) at).opt((Integer) k);
            else return null;
            if (at == null || at == JSONObject.NULL) return null;
        }
        return at;
    }

    private void set(List<Object> path, Object value) {
        try {
            Object at = scene;
            for (int i = 0; i < path.size() - 1; i++) {
                Object k = path.get(i), next;
                if (at instanceof JSONObject) {
                    JSONObject o = (JSONObject) at;
                    next = o.opt((String) k);
                    if (!(next instanceof JSONObject) && !(next instanceof JSONArray)) { next = new JSONObject(); o.put((String) k, next); }
                } else {
                    next = ((JSONArray) at).opt((Integer) k);
                    if (next == null) return;
                }
                at = next;
            }
            Object last = path.get(path.size() - 1);
            if (at instanceof JSONObject) ((JSONObject) at).put((String) last, value);
            else if (at instanceof JSONArray) ((JSONArray) at).put((Integer) last, value);
        } catch (JSONException | ClassCastException ignored) {
        }
    }

    /** A change: remember the scene as it was (for Undo) when `step`, make it, pass it on. */
    private void edit(List<Object> path, Object value, boolean step) {
        if (step) remember();
        set(path, value);
        host.changed(scene.toString());
    }

    private void remember() {
        undo.push(scene.toString());
        while (undo.size() > UNDO_MAX) undo.removeLast();
        redo.clear();
        undoButton.setAlpha(1f);
        redoButton.setAlpha(0.4f);
    }

    private void undo() { step(undo, redo); }
    private void redo() { step(redo, undo); }

    /** Go back (or forward again): the scene from `from`, keeping this one in `to`. */
    private void step(ArrayDeque<String> from, ArrayDeque<String> to) {
        String s = from.poll();
        if (s == null) return;
        to.push(scene.toString());
        while (to.size() > UNDO_MAX) to.removeLast();
        try { scene = new JSONObject(s); } catch (JSONException ignored) { }
        staged = null;
        host.changed(scene.toString());
        rebuild();
    }

    private static List<Object> at(List<Object> path, Object key) {
        List<Object> p = new ArrayList<>(path);
        p.add(key);
        return p;
    }

    private static String key(List<Object> path) {
        StringBuilder b = new StringBuilder();
        for (Object o : path) b.append(o).append('.');
        return b.toString();
    }

    // ── Building ─────────────────────────────────────────────────────────

    private void rebuild() {
        undoButton.setAlpha(undo.isEmpty() ? 0.4f : 1f);
        redoButton.setAlpha(redo.isEmpty() ? 0.4f : 1f);
        tabs.removeAllViews();
        List<String> labels = new ArrayList<>();
        List<Integer> index = new ArrayList<>();
        for (int i = 0; i < parts.size(); i++) {
            if (parts.get(i).optBoolean("adv") && !advanced) continue;
            labels.add(parts.get(i).optString("label"));
            index.add(i);
        }
        labels.add(advanced ? "Fewer settings" : "All settings");
        int sel = Math.max(0, index.indexOf(part));
        if (!index.contains(part)) part = index.isEmpty() ? 0 : index.get(0);
        Ui.Chips chips = new Ui.Chips(a, labels.toArray(new String[0]), sel, i -> {
            if (i == index.size()) { advanced = !advanced; rebuild(); return; }
            part = index.get(i);
            scroll.scrollTo(0, 0);
            rebuild();
        });
        ((ViewGroup) chips.view).removeView(chips.row);
        tabs.addView(chips.row);
        int y = scroll.getScrollY();
        body.removeAllViews();
        if (parts.isEmpty()) return;
        JSONObject p = parts.get(part);
        List<Object> path = new ArrayList<>();
        path.add(p.optString("key"));
        if (p.optString("key").equals("canvas")) canvas(body);
        else if (p.optString("kind").equals("list")) list(body, p, path, 0);
        else fields(body, p, path, 0);
        scroll.post(() -> scroll.scrollTo(0, y));
    }

    private void fields(LinearLayout into, JSONObject group, List<Object> path, int depth) {
        JSONArray fs = group.optJSONArray("fields");
        if (fs == null) return;
        for (int i = 0; i < fs.length(); i++) {
            JSONObject f = fs.optJSONObject(i);
            if (f == null || (f.optBoolean("adv") && !advanced)) continue;
            View v = field(f, at(path, f.optString("key")), depth);
            if (v != null) into.addView(v);
        }
    }

    private View field(JSONObject f, List<Object> path, int depth) {
        switch (f.optString("kind")) {
            case "bool": return bool(f, path);
            case "number": return number(f, path);
            case "enum": return choice(f, path);
            case "set": return set(f, path);
            case "color": return color(f, path);
            case "colors": return colors(f, path);
            case "pair": return pair(f, path);
            case "group": return group(f, path, depth);
            case "list": {
                LinearLayout box = new LinearLayout(a);
                box.setOrientation(LinearLayout.VERTICAL);
                list(box, f, path, depth);
                return box;
            }
            default: return null;
        }
    }

    /** A setting's name; long-press tells what it does. */
    private TextView name(JSONObject f) {
        TextView t = new TextView(a);
        t.setText(f.optString("label"));
        t.setTextColor(Ui.TEXT);
        t.setTextSize(TypedValue.COMPLEX_UNIT_SP, 15);
        String help = f.optString("help", "");
        if (!help.isEmpty()) t.setOnLongClickListener(v -> { explain(f.optString("label"), help); return true; });
        return t;
    }

    private void explain(String title, String help) {
        new AlertDialog.Builder(a).setTitle(title).setMessage(help.replace("`", "")).setPositiveButton("OK", null).show();
    }

    private LinearLayout row(JSONObject f) {
        LinearLayout r = new LinearLayout(a);
        r.setGravity(Gravity.CENTER_VERTICAL);
        r.setMinimumHeight(dp(48));
        r.addView(name(f), new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        String help = f.optString("help", "");
        if (!help.isEmpty()) r.setOnLongClickListener(v -> { explain(f.optString("label"), help); return true; });
        return r;
    }

    private View bool(JSONObject f, List<Object> path) {
        LinearLayout r = row(f);
        Switch s = new Switch(a);
        Object v = get(path);
        s.setChecked(v instanceof Boolean ? (Boolean) v : f.optBoolean("default"));
        s.setOnCheckedChangeListener((b, on) -> edit(path, on, true));
        r.addView(s);
        return r;
    }

    private static double num(Object v, double fallback) { return v instanceof Number ? ((Number) v).doubleValue() : fallback; }

    private static String fmt(double v, boolean integer, String unit) {
        String s = integer ? String.valueOf(Math.round(v)) : String.format(Locale.US, "%.3f", v).replaceAll("0+$", "").replaceAll("\\.$", "");
        return unit.isEmpty() ? s : s + " " + unit;
    }

    /** Most a typed number may be, either way, where a setting has no range of its own. */
    private static final double TYPED_LIMIT = 1e6;

    /** A number to type in: selectable, keeping to `lo`..`hi` (rounded for an integer); text that
     *  is not a number puts the value back. `commit` gets the value kept; it may say why it was
     *  changed. */
    private EditText numberField(double v, boolean integer, double lo, double hi, java.util.function.DoubleConsumer commit) {
        EditText e = new EditText(a);
        e.setTextColor(Ui.ACCENT);
        e.setSingleLine(true);
        e.setSelectAllOnFocus(true);
        e.setBackground(Ui.round(Color.argb(28, 255, 255, 255), dp(8), 0, 0));
        e.setPadding(dp(8), dp(4), dp(8), dp(4));
        e.setInputType(InputType.TYPE_CLASS_NUMBER | (integer ? 0 : InputType.TYPE_NUMBER_FLAG_DECIMAL) | (lo < 0 ? InputType.TYPE_NUMBER_FLAG_SIGNED : 0));
        e.setImeOptions(EditorInfo.IME_ACTION_DONE);
        e.setText(fmt(v, integer, ""));
        e.setMinWidth(dp(64));
        e.setGravity(Gravity.END);
        // (The value shown, kept in the tag: a slider beside it moves it too.)
        e.setTag(v);
        Runnable done = () -> {
            double shown = e.getTag() instanceof Double ? (Double) e.getTag() : v;
            String text = e.getText().toString().trim().replace(',', '.');
            double x;
            try { x = Double.parseDouble(text); } catch (NumberFormatException ex) { x = Double.NaN; }
            if (Double.isNaN(x) || Double.isInfinite(x)) {
                if (!text.isEmpty()) toast("“" + text + "” is not a number");
                e.setText(fmt(shown, integer, ""));
                return;
            }
            double kept = Math.max(lo, Math.min(hi, x));
            if (integer) kept = Math.round(kept);
            if (kept != x && Math.abs(kept - x) > 1e-9) toast("Kept within " + fmt(lo, integer, "") + " … " + fmt(hi, integer, ""));
            e.setText(fmt(kept, integer, ""));
            if (Math.abs(kept - shown) < 1e-12) return;
            e.setTag(kept);
            commit.accept(kept);
        };
        e.setOnEditorActionListener((t, id, ev) -> { done.run(); e.clearFocus(); hideKeyboard(e); return true; });
        e.setOnFocusChangeListener((t, has) -> { if (!has) done.run(); });
        return e;
    }

    private void hideKeyboard(View v) {
        android.view.inputmethod.InputMethodManager im = (android.view.inputmethod.InputMethodManager) a.getSystemService(android.content.Context.INPUT_METHOD_SERVICE);
        if (im != null) im.hideSoftInputFromWindow(v.getWindowToken(), 0);
    }

    private void toast(String s) { android.widget.Toast.makeText(a, s, android.widget.Toast.LENGTH_SHORT).show(); }

    private View number(JSONObject f, List<Object> path) {
        final boolean integer = f.optBoolean("int");
        final String unit = f.optString("unit", "").trim();
        final double v = num(get(path), f.optDouble("default", 0));
        LinearLayout box = new LinearLayout(a);
        box.setOrientation(LinearLayout.VERTICAL);
        LinearLayout r = row(f);
        box.addView(r);
        if (f.has("min") && f.has("max")) {
            final double lo = f.optDouble("min"), hi = f.optDouble("max");
            final int steps = integer && hi - lo <= 1000 ? (int) Math.round(hi - lo) : 1000;
            SeekBar s = new SeekBar(a);
            // The value: dragged with the slider, or typed in (kept to the slider's range).
            EditText val = numberField(v, integer, lo, hi, x -> {
                s.setProgress((int) Math.round(Math.max(0, Math.min(1, (x - lo) / (hi - lo))) * steps));
                edit(path, integer ? (Object) Math.round(x) : (Object) x, true);
            });
            r.addView(val);
            if (!unit.isEmpty()) {
                TextView u = new TextView(a);
                u.setText(" " + unit);
                u.setTextColor(Ui.DIM);
                r.addView(u);
            }
            s.setMax(steps);
            s.setProgress((int) Math.round(Math.max(0, Math.min(1, (v - lo) / (hi - lo))) * steps));
            s.getProgressDrawable().setTint(Ui.ACCENT);
            s.getThumb().setTint(Ui.ACCENT);
            s.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
                @Override public void onProgressChanged(SeekBar b, int p, boolean user) {
                    if (!user) return;
                    double x = lo + (hi - lo) * p / steps;
                    if (integer) x = Math.round(x);
                    val.setText(fmt(x, integer, ""));
                    val.setTag(x);
                    edit(path, integer ? (Object) Math.round(x) : (Object) x, false);
                }
                @Override public void onStartTrackingTouch(SeekBar b) { remember(); }
                @Override public void onStopTrackingTouch(SeekBar b) { }
            });
            box.addView(s, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(36)));
        } else {
            // No sensible slider: a number, typed or nudged with − and +.
            final double floor = f.has("min") ? f.optDouble("min") : -TYPED_LIMIT;
            EditText e = numberField(v, integer, floor, TYPED_LIMIT, x -> edit(path, integer ? (Object) Math.round(x) : (Object) x, true));
            for (int sign : new int[]{-1, 1}) {
                TextView b = Ui.button(a, sign < 0 ? "−" : "+", false, x -> {
                    double cur = num(get(path), v);
                    double step = integer ? 1 : (Math.abs(cur) >= 10 ? 1 : 0.1);
                    double n = Math.max(floor, Math.min(TYPED_LIMIT, cur + sign * step));
                    if (!integer) n = Math.round(n * 1000) / 1000.0;
                    e.setText(fmt(n, integer, ""));
                    e.setTag(n);
                    edit(path, integer ? (Object) Math.round(n) : (Object) n, true);
                });
                if (sign < 0) { r.addView(b); r.addView(e); } else r.addView(b);
            }
            if (!unit.isEmpty()) {
                TextView u = new TextView(a);
                u.setText(" " + unit);
                u.setTextColor(Ui.DIM);
                r.addView(u);
            }
        }
        return box;
    }

    private static SpannableStringBuilder withHelp(String label, String help) {
        SpannableStringBuilder b = new SpannableStringBuilder(label);
        if (help != null && !help.isEmpty() && !help.equals("null")) {
            int at = b.length();
            b.append("\n").append(help.replace("`", ""));
            b.setSpan(new RelativeSizeSpan(0.8f), at, b.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
            b.setSpan(new ForegroundColorSpan(Color.GRAY), at, b.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
        }
        return b;
    }

    private View choice(JSONObject f, List<Object> path) {
        LinearLayout r = row(f);
        JSONArray cs = f.optJSONArray("choices");
        if (cs == null) return null;
        String cur = get(path) instanceof String ? (String) get(path) : f.optString("default");
        TextView b = Ui.button(a, labelOf(cs, cur) + "  ▾", false, null);
        b.setOnClickListener(v -> {
            CharSequence[] items = new CharSequence[cs.length()];
            int sel = -1;
            String now = get(path) instanceof String ? (String) get(path) : f.optString("default");
            for (int i = 0; i < cs.length(); i++) {
                JSONObject c = cs.optJSONObject(i);
                items[i] = withHelp(c.optString("label"), c.optString("help", ""));
                if (c.optString("value").equals(now)) sel = i;
            }
            new AlertDialog.Builder(a).setTitle(f.optString("label")).setSingleChoiceItems(items, sel, (d, i) -> {
                String value = cs.optJSONObject(i).optString("value");
                edit(path, value, true);
                b.setText(labelOf(cs, value) + "  ▾");
                d.dismiss();
                // A list item's title names its kind.
                if (f.optString("key").equals("kind")) rebuild();
            }).show();
        });
        r.addView(b);
        return r;
    }

    private static String labelOf(JSONArray cs, String value) {
        for (int i = 0; i < cs.length(); i++) {
            JSONObject c = cs.optJSONObject(i);
            if (c != null && c.optString("value").equals(value)) return c.optString("label");
        }
        return value;
    }

    private View set(JSONObject f, List<Object> path) {
        LinearLayout r = row(f);
        JSONArray cs = f.optJSONArray("choices");
        if (cs == null) return null;
        TextView b = Ui.button(a, "", false, null);
        Runnable show = () -> {
            JSONArray have = get(path) instanceof JSONArray ? (JSONArray) get(path) : new JSONArray();
            StringBuilder s = new StringBuilder();
            for (int i = 0; i < have.length(); i++) { if (s.length() > 0) s.append(", "); s.append(labelOf(cs, have.optString(i))); }
            b.setText((s.length() == 0 ? "Any" : s.toString()) + "  ▾");
        };
        show.run();
        b.setOnClickListener(v -> {
            JSONArray have = get(path) instanceof JSONArray ? (JSONArray) get(path) : new JSONArray();
            CharSequence[] items = new CharSequence[cs.length()];
            boolean[] on = new boolean[cs.length()];
            for (int i = 0; i < cs.length(); i++) {
                JSONObject c = cs.optJSONObject(i);
                items[i] = withHelp(c.optString("label"), c.optString("help", ""));
                for (int k = 0; k < have.length(); k++) if (have.optString(k).equals(c.optString("value"))) on[i] = true;
            }
            new AlertDialog.Builder(a).setTitle(f.optString("label"))
                    .setMultiChoiceItems(items, on, (d, i, checked) -> on[i] = checked)
                    .setPositiveButton("OK", (d, w) -> {
                        JSONArray out = new JSONArray();
                        for (int i = 0; i < on.length; i++) if (on[i]) out.put(cs.optJSONObject(i).optString("value"));
                        edit(path, out, true);
                        show.run();
                    }).setNegativeButton("Cancel", null).show();
        });
        r.addView(b);
        return r;
    }

    private static int rgb(Object v, JSONArray fallback) {
        JSONArray c = v instanceof JSONArray ? (JSONArray) v : fallback;
        if (c == null || c.length() < 3) return 0;
        return Color.rgb(c.optInt(0), c.optInt(1), c.optInt(2));
    }

    private static JSONArray rgbArray(int col) { return new JSONArray().put(Color.red(col)).put(Color.green(col)).put(Color.blue(col)); }

    private View swatch(int col, View.OnClickListener on) {
        View s = new View(a);
        s.setBackground(Ui.round(Color.rgb(Color.red(col), Color.green(col), Color.blue(col)), dp(10), Color.argb(110, 255, 255, 255), dp(1)));
        s.setOnClickListener(on);
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(dp(44), dp(32));
        lp.leftMargin = dp(8);
        s.setLayoutParams(lp);
        return s;
    }

    private View color(JSONObject f, List<Object> path) {
        LinearLayout r = row(f);
        final View[] sw = new View[1];
        sw[0] = swatch(rgb(get(path), f.optJSONArray("default")), v -> ColorPicker.show(a, f.optString("label"), rgb(get(path), f.optJSONArray("default")), col -> {
            edit(path, rgbArray(col), true);
            sw[0].setBackground(Ui.round(0xFF000000 | col, dp(10), Color.argb(110, 255, 255, 255), dp(1)));
        }));
        r.addView(sw[0]);
        return r;
    }

    private View colors(JSONObject f, List<Object> path) {
        LinearLayout r = row(f);
        int n = f.optInt("count", 2);
        for (int i = 0; i < n; i++) {
            final int k = i;
            final View[] sw = new View[1];
            JSONArray d = f.optJSONArray("default");
            JSONArray dk = d == null ? null : d.optJSONArray(k);
            sw[0] = swatch(rgb(get(at(path, k)), dk), v -> ColorPicker.show(a, f.optString("label") + " " + (k + 1), rgb(get(at(path, k)), dk), col -> {
                try {
                    JSONArray all = get(path) instanceof JSONArray ? new JSONArray(get(path).toString()) : new JSONArray(d == null ? "[]" : d.toString());
                    all.put(k, rgbArray(col));
                    edit(path, all, true);
                } catch (JSONException ignored) {
                }
                sw[0].setBackground(Ui.round(0xFF000000 | col, dp(10), Color.argb(110, 255, 255, 255), dp(1)));
            }));
            r.addView(sw[0]);
        }
        return r;
    }

    private View pair(JSONObject f, List<Object> path) {
        LinearLayout box = new LinearLayout(a);
        box.setOrientation(LinearLayout.VERTICAL);
        box.addView(row(f));
        String[] axis = {"Across", "Down"};
        for (int i = 0; i < 2; i++) {
            final int k = i;
            LinearLayout r = new LinearLayout(a);
            r.setGravity(Gravity.CENTER_VERTICAL);
            TextView l = new TextView(a);
            l.setText(axis[i]);
            l.setTextColor(Ui.DIM);
            r.addView(l, new LinearLayout.LayoutParams(dp(64), ViewGroup.LayoutParams.WRAP_CONTENT));
            SeekBar s = new SeekBar(a);
            s.setMax(1000);
            JSONArray d = f.optJSONArray("default");
            Object cur = get(path);
            double v = cur instanceof JSONArray ? ((JSONArray) cur).optDouble(k, 0.5) : d == null ? 0.5 : d.optDouble(k, 0.5);
            s.setProgress((int) Math.round(Math.max(0, Math.min(1, v)) * 1000));
            s.getProgressDrawable().setTint(Ui.ACCENT);
            s.getThumb().setTint(Ui.ACCENT);
            s.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
                @Override public void onProgressChanged(SeekBar b, int p, boolean user) {
                    if (!user) return;
                    try {
                        Object now = get(path);
                        JSONArray xy = now instanceof JSONArray ? new JSONArray(now.toString()) : new JSONArray(d == null ? "[0.5,0.5]" : d.toString());
                        xy.put(k, p / 1000.0);
                        edit(path, xy, false);
                    } catch (JSONException ignored) {
                    }
                }
                @Override public void onStartTrackingTouch(SeekBar b) { remember(); }
                @Override public void onStopTrackingTouch(SeekBar b) { }
            });
            r.addView(s, new LinearLayout.LayoutParams(0, dp(36), 1f));
            // Or typed in, 0 … 1.
            r.addView(numberField(v, false, 0, 1, x -> {
                s.setProgress((int) Math.round(x * 1000));
                try {
                    Object now = get(path);
                    JSONArray xy = now instanceof JSONArray ? new JSONArray(now.toString()) : new JSONArray(d == null ? "[0.5,0.5]" : d.toString());
                    xy.put(k, x);
                    edit(path, xy, true);
                } catch (JSONException ignored) {
                }
            }));
            box.addView(r);
        }
        return box;
    }

    /** A struct's settings under a header that opens and closes it; its `enabled` switch, if it has
     *  one, sits in the header. */
    private View group(JSONObject f, List<Object> path, int depth) {
        LinearLayout box = new LinearLayout(a);
        box.setOrientation(LinearLayout.VERTICAL);
        box.setPadding(0, dp(4), 0, dp(4));
        box.addView(header(f.optString("label"), f, path, depth, null));
        if (open.contains(key(path))) {
            LinearLayout inner = indent(depth);
            fields(inner, withoutEnabled(f), path, depth + 1);
            box.addView(inner);
        }
        return box;
    }

    private LinearLayout indent(int depth) {
        LinearLayout inner = new LinearLayout(a);
        inner.setOrientation(LinearLayout.VERTICAL);
        inner.setPadding(dp(12), 0, 0, dp(8));
        inner.setBackground(Ui.round(Color.argb(depth % 2 == 0 ? 18 : 10, 255, 255, 255), dp(12), 0, 0));
        return inner;
    }

    private static JSONObject withoutEnabled(JSONObject f) {
        try {
            JSONObject copy = new JSONObject(f.toString());
            JSONArray fs = copy.optJSONArray("fields");
            if (fs == null) return copy;
            JSONArray kept = new JSONArray();
            for (int i = 0; i < fs.length(); i++) if (!fs.optJSONObject(i).optString("key").equals("enabled")) kept.put(fs.optJSONObject(i));
            copy.put("fields", kept);
            return copy;
        } catch (JSONException e) {
            return f;
        }
    }

    private static boolean hasEnabled(JSONObject f) {
        JSONArray fs = f.optJSONArray("fields");
        if (fs == null) return false;
        for (int i = 0; i < fs.length(); i++) if (fs.optJSONObject(i).optString("key").equals("enabled")) return true;
        return false;
    }

    /** A header row: ▸/▾, the title, the `enabled` switch if any, and extra buttons. */
    private View header(String title, JSONObject f, List<Object> path, int depth, View extra) {
        LinearLayout r = new LinearLayout(a);
        r.setGravity(Gravity.CENTER_VERTICAL);
        r.setMinimumHeight(dp(48));
        boolean isOpen = open.contains(key(path));
        TextView t = new TextView(a);
        t.setText((isOpen ? "▾  " : "▸  ") + title);
        t.setTextColor(Ui.TEXT);
        t.setTextSize(TypedValue.COMPLEX_UNIT_SP, depth == 0 ? 16 : 15);
        t.setTypeface(Typeface.DEFAULT_BOLD);
        r.addView(t, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        View.OnClickListener toggle = v -> {
            if (!open.remove(key(path))) open.add(key(path));
            rebuild();
        };
        r.setOnClickListener(toggle);
        String help = f.optString("help", "");
        if (!help.isEmpty()) r.setOnLongClickListener(v -> { explain(title, help); return true; });
        if (extra != null) r.addView(extra);
        if (hasEnabled(f)) {
            Switch s = new Switch(a);
            List<Object> ep = at(path, "enabled");
            Object v = get(ep);
            s.setChecked(v instanceof Boolean && (Boolean) v);
            s.setOnCheckedChangeListener((b, on) -> {
                edit(ep, on, true);
                if (on && open.add(key(path))) rebuild();
            });
            r.addView(s);
        }
        return r;
    }

    /** A list: its items, each opening to its settings, with duplicate and remove; and Add. */
    private void list(LinearLayout into, JSONObject f, List<Object> path, int depth) {
        JSONArray items = get(path) instanceof JSONArray ? (JSONArray) get(path) : new JSONArray();
        JSONObject item = f.optJSONObject("item");
        String one = f.optString("one", "item");
        String One = one.isEmpty() ? "Item" : Character.toUpperCase(one.charAt(0)) + one.substring(1);
        LinearLayout head = new LinearLayout(a);
        head.setGravity(Gravity.CENTER_VERTICAL);
        head.setMinimumHeight(dp(48));
        TextView t = new TextView(a);
        t.setText(f.optString("label") + "  (" + items.length() + ")");
        t.setTextColor(Ui.TEXT);
        t.setTypeface(Typeface.DEFAULT_BOLD);
        t.setTextSize(TypedValue.COMPLEX_UNIT_SP, 16);
        head.addView(t, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        head.addView(Ui.button(a, "+ Add " + one, false, v -> {
            try {
                JSONArray next = new JSONArray(items.toString());
                next.put(new JSONObject(f.optJSONObject("new") == null ? "{}" : f.optJSONObject("new").toString()));
                edit(path, next, true);
                open.add(key(at(path, next.length() - 1)));
                rebuild();
            } catch (JSONException ignored) {
            }
        }));
        into.addView(head);
        if (item == null) return;
        for (int i = 0; i < items.length(); i++) {
            final int k = i;
            List<Object> ip = at(path, i);
            JSONObject it = items.optJSONObject(i);
            String kind = it == null ? "" : it.optString("kind", "");
            JSONObject kindField = findField(item, "kind");
            String title = One + " " + (i + 1) + (kind.isEmpty() ? "" : " · " + (kindField == null || kindField.optJSONArray("choices") == null ? kind : labelOf(kindField.optJSONArray("choices"), kind)));
            LinearLayout extra = new LinearLayout(a);
            extra.addView(Ui.button(a, "Copy", false, v -> {
                try {
                    JSONArray next = new JSONArray();
                    for (int j = 0; j < items.length(); j++) {
                        next.put(new JSONObject(items.get(j).toString()));
                        if (j == k) next.put(new JSONObject(items.get(j).toString()));
                    }
                    edit(path, next, true);
                    rebuild();
                } catch (JSONException ignored) {
                }
            }));
            extra.addView(gap());
            extra.addView(Ui.button(a, "Remove", false, v -> {
                JSONArray next = new JSONArray();
                for (int j = 0; j < items.length(); j++) if (j != k) next.put(items.opt(j));
                open.remove(key(ip));
                edit(path, next, true);
                rebuild();
            }));
            LinearLayout box = new LinearLayout(a);
            box.setOrientation(LinearLayout.VERTICAL);
            box.addView(header(title, item, ip, depth + 1, extra));
            if (open.contains(key(ip))) {
                LinearLayout inner = indent(depth + 1);
                fields(inner, withoutEnabled(item), ip, depth + 2);
                box.addView(inner);
            }
            into.addView(box);
        }
    }

    // ── The canvas ───────────────────────────────────────────────────────

    /** Smallest and largest side, and most pixels, a canvas may have: what a phone can draw. */
    static final int CANVAS_MIN = 64, CANVAS_MAX = 4096, CANVAS_PIXELS = 4096 * 2160;
    /** The size being chosen, before Apply (null: the scene's own). */
    private int[] staged;

    private int[] canvasNow() {
        Object w = get(java.util.Arrays.asList("canvas", "width")), h = get(java.util.Arrays.asList("canvas", "height"));
        return new int[]{w instanceof Number ? ((Number) w).intValue() : 480, h instanceof Number ? ((Number) h).intValue() : 854};
    }

    /** Why a canvas of `w` x `h` cannot be used, or null. */
    static String canvasProblem(int w, int h) {
        if (w < CANVAS_MIN || h < CANVAS_MIN) return "Each side needs at least " + CANVAS_MIN + " px";
        if (w > CANVAS_MAX || h > CANVAS_MAX) return "Each side can be at most " + CANVAS_MAX + " px";
        if ((long) w * h > CANVAS_PIXELS) return String.format(Locale.US, "%.1f megapixels is more than a phone can draw (at most %.1f)", w * (double) h / 1e6, CANVAS_PIXELS / 1e6);
        return null;
    }

    private static int gcd(int a, int b) { return b == 0 ? a : gcd(b, a % b); }

    /** "16:9 landscape", "9:19.5 portrait", "1:1 square". */
    static String shape(int w, int h) {
        String o = w == h ? "square" : w > h ? "landscape" : "portrait";
        int g = gcd(w, h);
        String ratio;
        if (w / g <= 32 && h / g <= 32) ratio = (w / g) + ":" + (h / g);
        else if (w <= h) ratio = String.format(Locale.US, "9:%.1f", 9.0 * h / w).replace(".0", "");
        else ratio = String.format(Locale.US, "%.1f:9", 9.0 * w / h).replace(".0", "");
        return ratio + " " + o;
    }

    /** The canvas: its size typed in or picked, turned between portrait and landscape, and only
     *  used once Apply is pressed; and what the preview shows. */
    private void canvas(LinearLayout into) {
        final int[] now = canvasNow();
        if (staged == null) staged = now.clone();
        TextView title = Ui.label(a, "Size (pixels)");
        into.addView(title);

        LinearLayout size = new LinearLayout(a);
        size.setGravity(Gravity.CENTER_VERTICAL);
        size.setMinimumHeight(dp(52));
        TextView info = new TextView(a);
        info.setTextSize(TypedValue.COMPLEX_UNIT_SP, 13);
        info.setPadding(0, dp(2), 0, dp(6));
        TextView apply = Ui.button(a, "Apply", true, null), revert = Ui.button(a, "Revert", false, null);
        final EditText[] fields = new EditText[2];
        Runnable show = () -> {
            String why = canvasProblem(staged[0], staged[1]);
            boolean changed = staged[0] != now[0] || staged[1] != now[1];
            if (why != null) { info.setText(why); info.setTextColor(Color.rgb(255, 120, 110)); }
            else {
                info.setText(String.format(Locale.US, "%s · %.1f MP%s", shape(staged[0], staged[1]), staged[0] * (double) staged[1] / 1e6,
                        changed ? String.format(Locale.US, "  (now %d × %d: press Apply)", now[0], now[1]) : ""));
                info.setTextColor(changed ? Ui.ACCENT : Ui.DIM);
            }
            apply.setAlpha(changed && why == null ? 1f : 0.4f);
            revert.setAlpha(changed ? 1f : 0.4f);
            for (int i = 0; i < 2; i++) if (fields[i] != null && !fields[i].hasFocus()) { fields[i].setText(String.valueOf(staged[i])); fields[i].setTag((double) staged[i]); }
        };
        for (int i = 0; i < 2; i++) {
            final int k = i;
            // Typed in: anything from 16 to 8192 is taken here, and Apply says whether it can be used.
            fields[i] = numberField(staged[i], true, 16, 8192, x -> { staged[k] = (int) Math.round(x); show.run(); });
            size.addView(fields[i], new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
            if (i == 0) {
                TextView by = new TextView(a);
                by.setText("  ×  ");
                by.setTextColor(Ui.TEXT);
                size.addView(by);
            }
        }
        size.addView(gap());
        size.addView(Ui.button(a, "⇄", false, v -> { staged = new int[]{staged[1], staged[0]}; show.run(); }));
        into.addView(size);
        into.addView(info);

        // Picks.
        int[] scr = host.screen();
        // (Portrait and Landscape turn whatever size is chosen now.)
        String[] names = {"Portrait", "Landscape", "This screen", "Square", "720 × 1280", "1280 × 720", "1080 × 1920", "1920 × 1080"};
        int[][] fixed = {null, null, {scr[0], scr[1]}, {1080, 1080}, {720, 1280}, {1280, 720}, {1080, 1920}, {1920, 1080}};
        android.widget.HorizontalScrollView hs = new android.widget.HorizontalScrollView(a);
        hs.setHorizontalScrollBarEnabled(false);
        LinearLayout picks = new LinearLayout(a);
        for (int i = 0; i < names.length; i++) {
            final int k = i;
            picks.addView(Ui.button(a, names[i], false, v -> {
                int lo = Math.min(staged[0], staged[1]), hi = Math.max(staged[0], staged[1]);
                staged = k == 0 ? new int[]{lo, hi} : k == 1 ? new int[]{hi, lo} : fixed[k].clone();
                show.run();
            }));
            picks.addView(gap());
        }
        hs.addView(picks);
        into.addView(hs);

        LinearLayout act = new LinearLayout(a);
        act.setPadding(0, dp(10), 0, dp(4));
        revert.setOnClickListener(v -> { staged = canvasNow(); show.run(); });
        apply.setOnClickListener(v -> {
            for (EditText f : fields) f.clearFocus();
            hideKeyboard(v);
            String why = canvasProblem(staged[0], staged[1]);
            if (why != null) { toast(why); return; }
            int[] cur = canvasNow();
            if (cur[0] == staged[0] && cur[1] == staged[1]) return;
            try {
                remember();
                JSONObject c = scene.optJSONObject("canvas") == null ? new JSONObject() : new JSONObject(scene.optJSONObject("canvas").toString());
                c.put("width", staged[0]).put("height", staged[1]);
                set(java.util.Collections.singletonList("canvas"), c);
                host.changed(scene.toString());
                toast("Canvas is now " + staged[0] + " × " + staged[1]);
            } catch (JSONException ignored) {
            }
            rebuild();
        });
        act.addView(revert, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        act.addView(gap());
        act.addView(apply, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.4f));
        into.addView(act);
        show.run();

        into.addView(Ui.label(a, "The preview shows"));
        into.addView(new Ui.Chips(a, new String[]{"The canvas's shape", "The whole screen"}, host.canvasPreview() ? 0 : 1, i -> host.setCanvasPreview(i == 0)).view);
        TextView note = new TextView(a);
        note.setText("Pictures and GIF/WebP animations are made at the canvas's size and shape. A wallpaper always fills the screen. The camera's horizon and zoom are shares of the canvas's height, so a landscape canvas sees more to the sides.");
        note.setTextColor(Ui.DIM);
        note.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
        note.setPadding(0, dp(8), 0, 0);
        into.addView(note);
    }

    private static JSONObject findField(JSONObject group, String key) {
        JSONArray fs = group.optJSONArray("fields");
        if (fs == null) return null;
        for (int i = 0; i < fs.length(); i++) if (fs.optJSONObject(i).optString("key").equals(key)) return fs.optJSONObject(i);
        return null;
    }

    private View gap() {
        View g = new View(a);
        g.setLayoutParams(new LinearLayout.LayoutParams(dp(8), 1));
        return g;
    }

    private int dp(float v) { return Ui.dp(a, v); }
}

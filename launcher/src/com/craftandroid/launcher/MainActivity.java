package com.craftandroid.launcher;

import android.app.Activity;
import android.app.AlertDialog;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.SharedPreferences;
import android.content.pm.PackageInfo;
import android.content.pm.PackageInstaller;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.text.InputType;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.EditText;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Craft Launcher: lists the Craft ports, shows which are installed and whether a newer build is
 * on GitHub, and installs or updates them. Builds are GitHub releases of the configured
 * repository, tagged "<app>-b<versionCode>" with one APK asset each (see .github/workflows).
 */
public class MainActivity extends Activity {

    /** The apps this launcher manages: key (tag prefix), package, display name, blurb. */
    static final String[][] APPS = {
            {"lightcraft", "com.craftandroid.lightcraft", "LightCraft", "Non-destructive photo editing"},
            {"filmcraft", "com.craftandroid.filmcraft", "FilmCraft", "Video editing"},
            {"launcher", "com.craftandroid.launcher", "Craft Launcher", "This app"},
    };

    static final String ACTION_INSTALLED = "com.craftandroid.launcher.INSTALLED";

    final ExecutorService io = Executors.newSingleThreadExecutor();
    final Handler main = new Handler(Looper.getMainLooper());
    LinearLayout list;
    TextView status;
    SharedPreferences prefs;
    final List<Release> latest = new ArrayList<>();
    String error = null;
    boolean loading = false;
    String busyKey = null;
    int busyPercent = 0;

    static class Release {
        String key, tag, title, notes, apkUrl;
        long versionCode;
    }

    // ---- lifecycle ---------------------------------------------------------------------------

    @Override
    protected void onCreate(Bundle b) {
        super.onCreate(b);
        prefs = getSharedPreferences("launcher", MODE_PRIVATE);
        getWindow().setStatusBarColor(Color.rgb(18, 18, 20));
        getWindow().setNavigationBarColor(Color.rgb(18, 18, 20));

        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackgroundColor(Color.rgb(18, 18, 20));
        root.setPadding(dp(16), dp(20), dp(16), dp(8));
        root.setFitsSystemWindows(true);

        LinearLayout header = new LinearLayout(this);
        header.setGravity(Gravity.CENTER_VERTICAL);
        TextView title = text("Craft", 28, Color.WHITE, true);
        header.addView(title, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        Button refresh = button("Check", false);
        refresh.setOnClickListener(v -> refresh());
        header.addView(refresh);
        Button settings = button("⋯", false);
        settings.setOnClickListener(v -> settings());
        header.addView(settings);
        root.addView(header);

        status = text("", 13, Color.rgb(150, 150, 160), false);
        status.setPadding(0, dp(4), 0, dp(12));
        root.addView(status);

        ScrollView scroll = new ScrollView(this);
        list = new LinearLayout(this);
        list.setOrientation(LinearLayout.VERTICAL);
        scroll.addView(list);
        root.addView(scroll, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1));
        setContentView(root);

        IntentFilter f = new IntentFilter(ACTION_INSTALLED);
        if (Build.VERSION.SDK_INT >= 33) registerReceiver(installResult, f, Context.RECEIVER_NOT_EXPORTED);
        else registerReceiver(installResult, f);

        render();
        refresh();
    }

    @Override
    protected void onResume() {
        super.onResume();
        render(); // an app may have been installed or removed meanwhile
    }

    @Override
    protected void onDestroy() {
        super.onDestroy();
        try {
            unregisterReceiver(installResult);
        } catch (Exception ignored) {
        }
    }

    // ---- data --------------------------------------------------------------------------------

    String repo() {
        String r = prefs.getString("repo", null);
        if (r == null || r.trim().isEmpty()) r = getString(R.string.default_repo);
        return r.trim();
    }

    long installedCode(String pkg) {
        try {
            PackageInfo pi = getPackageManager().getPackageInfo(pkg, 0);
            return Build.VERSION.SDK_INT >= 28 ? pi.getLongVersionCode() : pi.versionCode;
        } catch (PackageManager.NameNotFoundException e) {
            return -1;
        }
    }

    String installedName(String pkg) {
        try {
            return getPackageManager().getPackageInfo(pkg, 0).versionName;
        } catch (PackageManager.NameNotFoundException e) {
            return null;
        }
    }

    Release latestOf(String key) {
        for (Release r : latest) if (r.key.equals(key)) return r;
        return null;
    }

    void refresh() {
        if (loading) return;
        loading = true;
        error = null;
        render();
        io.execute(() -> {
            List<Release> found = new ArrayList<>();
            String err = null;
            try {
                String body = get("https://api.github.com/repos/" + repo() + "/releases?per_page=60");
                JSONArray arr = new JSONArray(body);
                for (int i = 0; i < arr.length(); i++) {
                    JSONObject o = arr.getJSONObject(i);
                    if (o.optBoolean("draft")) continue;
                    String tag = o.optString("tag_name");
                    int b = tag.lastIndexOf("-b");
                    if (b <= 0) continue;
                    String key = tag.substring(0, b);
                    long code;
                    try {
                        code = Long.parseLong(tag.substring(b + 2));
                    } catch (NumberFormatException e) {
                        continue;
                    }
                    String apk = null;
                    JSONArray assets = o.optJSONArray("assets");
                    for (int k = 0; assets != null && k < assets.length(); k++) {
                        JSONObject a = assets.getJSONObject(k);
                        if (a.optString("name").endsWith(".apk")) apk = a.optString("browser_download_url");
                    }
                    if (apk == null) continue;
                    Release prev = null;
                    for (Release r : found) if (r.key.equals(key)) prev = r;
                    if (prev != null && prev.versionCode >= code) continue;
                    if (prev != null) found.remove(prev);
                    Release r = new Release();
                    r.key = key;
                    r.tag = tag;
                    r.versionCode = code;
                    r.title = o.optString("name", tag);
                    r.notes = o.optString("body", "");
                    r.apkUrl = apk;
                    found.add(r);
                }
            } catch (Exception e) {
                err = e.getMessage();
            }
            final String fe = err;
            main.post(() -> {
                loading = false;
                error = fe;
                if (fe == null) {
                    latest.clear();
                    latest.addAll(found);
                    prefs.edit().putLong("checked", System.currentTimeMillis()).apply();
                }
                render();
            });
        });
    }

    String get(String url) throws Exception {
        HttpURLConnection c = (HttpURLConnection) new URL(url).openConnection();
        c.setRequestProperty("Accept", "application/vnd.github+json");
        c.setRequestProperty("User-Agent", "craft-launcher");
        String token = prefs.getString("token", "");
        if (!token.isEmpty()) c.setRequestProperty("Authorization", "Bearer " + token);
        c.setConnectTimeout(15000);
        c.setReadTimeout(20000);
        int code = c.getResponseCode();
        if (code == 404) throw new Exception("repository " + repo() + " not found (private? add a token in ⋯)");
        if (code >= 400) throw new Exception("GitHub answered " + code);
        try (InputStream in = c.getInputStream()) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buf = new byte[1 << 14];
            int n;
            while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
            return out.toString("UTF-8");
        }
    }

    // ---- UI ----------------------------------------------------------------------------------

    void render() {
        if (list == null) return;
        if (loading) status.setText("Checking " + repo() + "…");
        else if (error != null) status.setText("Couldn't check for updates: " + error);
        else {
            long t = prefs.getLong("checked", 0);
            status.setText(t == 0 ? repo() : repo() + " · checked " + android.text.format.DateUtils.getRelativeTimeSpanString(t));
        }
        list.removeAllViews();
        for (String[] app : APPS) list.addView(card(app));
        TextView foot = text("Builds track the upstream Craft repositories automatically: when upstream changes, a new build appears here.", 12, Color.rgb(120, 120, 130), false);
        foot.setPadding(dp(4), dp(16), dp(4), dp(24));
        list.addView(foot);
    }

    View card(String[] app) {
        String key = app[0], pkg = app[1], name = app[2], blurb = app[3];
        long have = installedCode(pkg);
        Release r = latestOf(key);
        boolean self = pkg.equals(getPackageName());

        LinearLayout card = new LinearLayout(this);
        card.setOrientation(LinearLayout.HORIZONTAL);
        card.setGravity(Gravity.CENTER_VERTICAL);
        card.setPadding(dp(14), dp(14), dp(14), dp(14));
        GradientDrawable bg = new GradientDrawable();
        bg.setColor(Color.rgb(32, 32, 37));
        bg.setCornerRadius(dp(16));
        card.setBackground(bg);
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        lp.bottomMargin = dp(10);
        card.setLayoutParams(lp);

        ImageView icon = new ImageView(this);
        Drawable d = null;
        try {
            d = getPackageManager().getApplicationIcon(pkg);
        } catch (PackageManager.NameNotFoundException ignored) {
        }
        if (d != null) icon.setImageDrawable(d);
        else {
            GradientDrawable ph = new GradientDrawable();
            ph.setColor(Color.rgb(60, 60, 70));
            ph.setCornerRadius(dp(12));
            icon.setImageDrawable(ph);
        }
        card.addView(icon, new LinearLayout.LayoutParams(dp(52), dp(52)));

        LinearLayout mid = new LinearLayout(this);
        mid.setOrientation(LinearLayout.VERTICAL);
        mid.setPadding(dp(14), 0, dp(8), 0);
        mid.addView(text(name, 17, Color.WHITE, true));
        String state;
        if (have < 0) state = r == null ? "Not built yet" : "Not installed · build " + r.versionCode;
        else if (r != null && r.versionCode > have) state = "Update: build " + have + " → " + r.versionCode;
        else state = "Installed · " + installedName(pkg) + " (build " + have + ")";
        mid.addView(text(blurb, 13, Color.rgb(150, 150, 160), false));
        TextView st = text(state, 12, Color.rgb(120, 170, 255), false);
        mid.addView(st);
        if (key.equals(busyKey)) {
            ProgressBar pb = new ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal);
            pb.setMax(100);
            pb.setProgress(busyPercent);
            mid.addView(pb);
        }
        card.addView(mid, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));

        Button act = null;
        if (r != null && (have < 0 || r.versionCode > have)) {
            act = button(have < 0 ? "Install" : "Update", true);
            final Release rel = r;
            act.setOnClickListener(v -> install(rel));
            act.setEnabled(busyKey == null);
        } else if (have >= 0 && !self) {
            act = button("Open", false);
            act.setOnClickListener(v -> {
                Intent i = getPackageManager().getLaunchIntentForPackage(pkg);
                if (i != null) startActivity(i);
            });
        }
        if (act != null) card.addView(act);
        if (r != null && !r.notes.isEmpty()) {
            final Release rel = r;
            card.setOnClickListener(v -> new AlertDialog.Builder(this).setTitle(rel.title).setMessage(rel.notes).setPositiveButton(android.R.string.ok, null).show());
        }
        return card;
    }

    void settings() {
        LinearLayout box = new LinearLayout(this);
        box.setOrientation(LinearLayout.VERTICAL);
        box.setPadding(dp(20), dp(8), dp(20), 0);
        box.addView(text("GitHub repository (owner/name)", 13, Color.GRAY, false));
        final EditText repo = new EditText(this);
        repo.setText(repo());
        repo.setSingleLine(true);
        box.addView(repo);
        box.addView(text("Token (only for a private repository)", 13, Color.GRAY, false));
        final EditText token = new EditText(this);
        token.setText(prefs.getString("token", ""));
        token.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD);
        token.setSingleLine(true);
        box.addView(token);
        new AlertDialog.Builder(this).setTitle("Settings").setView(box)
                .setPositiveButton(android.R.string.ok, (dlg, w) -> {
                    prefs.edit().putString("repo", repo.getText().toString().trim()).putString("token", token.getText().toString().trim()).apply();
                    latest.clear();
                    refresh();
                })
                .setNegativeButton(android.R.string.cancel, null).show();
    }

    // ---- install -----------------------------------------------------------------------------

    void install(Release r) {
        if (Build.VERSION.SDK_INT >= 26 && !getPackageManager().canRequestPackageInstalls()) {
            new AlertDialog.Builder(this)
                    .setTitle("Allow installs")
                    .setMessage("To install the Craft apps, allow this launcher to install apps. You'll come back here afterwards.")
                    .setPositiveButton("Settings", (dlg, w) -> startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + getPackageName()))))
                    .setNegativeButton(android.R.string.cancel, null).show();
            return;
        }
        busyKey = r.key;
        busyPercent = 0;
        render();
        io.execute(() -> {
            File apk = new File(getCacheDir(), r.key + "-" + r.versionCode + ".apk");
            try {
                download(r.apkUrl, apk);
                main.post(() -> {
                    busyPercent = 100;
                    render();
                });
                PackageInstaller pi = getPackageManager().getPackageInstaller();
                PackageInstaller.SessionParams params = new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                if (Build.VERSION.SDK_INT >= 31) params.setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED);
                int id = pi.createSession(params);
                try (PackageInstaller.Session s = pi.openSession(id)) {
                    try (InputStream in = new FileInputStream(apk); OutputStream out = s.openWrite("app.apk", 0, apk.length())) {
                        byte[] buf = new byte[1 << 16];
                        int n;
                        while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
                        s.fsync(out);
                    }
                    Intent i = new Intent(ACTION_INSTALLED).setPackage(getPackageName());
                    int flags = PendingIntent.FLAG_UPDATE_CURRENT | (Build.VERSION.SDK_INT >= 31 ? PendingIntent.FLAG_MUTABLE : 0);
                    PendingIntent p = PendingIntent.getBroadcast(this, id, i, flags);
                    s.commit(p.getIntentSender());
                }
            } catch (Exception e) {
                main.post(() -> {
                    busyKey = null;
                    toast("Install failed: " + e.getMessage());
                    render();
                });
            }
        });
    }

    void download(String url, File to) throws Exception {
        String u = url;
        for (int hop = 0; hop < 6; hop++) {
            HttpURLConnection c = (HttpURLConnection) new URL(u).openConnection();
            c.setInstanceFollowRedirects(false);
            c.setRequestProperty("User-Agent", "craft-launcher");
            c.setRequestProperty("Accept", "application/octet-stream");
            String token = prefs.getString("token", "");
            if (!token.isEmpty() && u.contains("github.com")) c.setRequestProperty("Authorization", "Bearer " + token);
            int code = c.getResponseCode();
            if (code >= 300 && code < 400) {
                u = c.getHeaderField("Location");
                continue;
            }
            if (code >= 400) throw new Exception("download answered " + code);
            long total = c.getContentLengthLong();
            try (InputStream in = c.getInputStream(); OutputStream out = new FileOutputStream(to)) {
                byte[] buf = new byte[1 << 16];
                long got = 0;
                int n, last = -1;
                while ((n = in.read(buf)) > 0) {
                    out.write(buf, 0, n);
                    got += n;
                    if (total > 0) {
                        int pct = (int) (got * 100 / total);
                        if (pct != last) {
                            last = pct;
                            main.post(() -> {
                                busyPercent = pct;
                                render();
                            });
                        }
                    }
                }
            }
            return;
        }
        throw new Exception("too many redirects");
    }

    final BroadcastReceiver installResult = new BroadcastReceiver() {
        @Override
        public void onReceive(Context ctx, Intent i) {
            int st = i.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
            if (st == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                Intent confirm = i.getParcelableExtra(Intent.EXTRA_INTENT);
                if (confirm != null) {
                    confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                    startActivity(confirm);
                }
                return;
            }
            busyKey = null;
            if (st == PackageInstaller.STATUS_SUCCESS) toast("Installed");
            else toast("Not installed: " + i.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE));
            render();
        }
    };

    // ---- helpers -----------------------------------------------------------------------------

    int dp(float v) {
        return (int) TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, getResources().getDisplayMetrics());
    }

    TextView text(String s, float sp, int color, boolean bold) {
        TextView t = new TextView(this);
        t.setText(s);
        t.setTextSize(sp);
        t.setTextColor(color);
        if (bold) t.setTypeface(Typeface.DEFAULT_BOLD);
        return t;
    }

    Button button(String label, boolean primary) {
        Button b = new Button(this);
        b.setText(label);
        b.setAllCaps(false);
        b.setTextColor(Color.WHITE);
        GradientDrawable bg = new GradientDrawable();
        bg.setColor(primary ? Color.rgb(64, 156, 255) : Color.rgb(48, 48, 56));
        bg.setCornerRadius(dp(20));
        b.setBackground(bg);
        b.setMinHeight(dp(40));
        b.setMinimumHeight(dp(40));
        b.setPadding(dp(16), 0, dp(16), 0);
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, dp(40));
        lp.leftMargin = dp(6);
        b.setLayoutParams(lp);
        return b;
    }

    void toast(String s) {
        Toast.makeText(this, s, Toast.LENGTH_LONG).show();
    }
}

package com.craftandroid.kit;

import android.app.NativeActivity;
import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.graphics.Insets;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.provider.MediaStore;
import android.provider.OpenableColumns;
import android.view.View;
import android.view.WindowInsets;
import android.widget.Toast;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * The activity every Craft port runs in: a NativeActivity (the Rust app draws the whole window)
 * plus the few things only Java can do: the system file picker, saving to the shared gallery,
 * toasts, window insets and the back button. The Rust side is craft-mobile-kit/src/platform.rs.
 */
public class CraftActivity extends NativeActivity {
    private static final int PICK = 4101;

    static native void nativeOnDirs(String filesDir, String cacheDir);
    static native void nativeOnPicked(String path, String name);
    static native void nativeOnSaved(boolean ok, String message);
    static native void nativeOnInsets(int left, int top, int right, int bottom);
    static native void nativeOnBack();
    static native void nativeOnText(String id, String value);

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // NativeActivity dlopen()s the library itself, but JNI only finds native methods in
        // libraries loaded through System.loadLibrary.
        try {
            ActivityInfo ai = getPackageManager().getActivityInfo(getComponentName(), PackageManager.GET_META_DATA);
            System.loadLibrary(ai.metaData.getString("android.app.lib_name"));
        } catch (PackageManager.NameNotFoundException e) {
            throw new RuntimeException(e);
        }
        super.onCreate(savedInstanceState);
        nativeOnDirs(getFilesDir().getAbsolutePath(), getCacheDir().getAbsolutePath());
        // draw edge to edge on every Android version (15+ forces it); the app keeps its controls
        // out of the system bars with the insets reported below
        getWindow().setStatusBarColor(0);
        getWindow().setNavigationBarColor(0);
        View decor = getWindow().getDecorView();
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
        } else {
            decor.setSystemUiVisibility(View.SYSTEM_UI_FLAG_LAYOUT_STABLE | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
        decor.setOnApplyWindowInsetsListener((v, insets) -> {
            if (Build.VERSION.SDK_INT >= 30) {
                Insets i = insets.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                nativeOnInsets(i.left, i.top, i.right, i.bottom);
            } else {
                nativeOnInsets(insets.getSystemWindowInsetLeft(), insets.getSystemWindowInsetTop(),
                        insets.getSystemWindowInsetRight(), insets.getSystemWindowInsetBottom());
            }
            return v.onApplyWindowInsets(insets);
        });
        decor.requestApplyInsets();
    }

    @Override
    public void onBackPressed() {
        // the app decides: close a panel, leave the editor, or go to the background
        nativeOnBack();
    }

    // ---- called from Rust ------------------------------------------------------------------

    public void pickFile(final String mimes) {
        runOnUiThread(() -> {
            Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            i.addCategory(Intent.CATEGORY_OPENABLE);
            String[] types = mimes.split(",");
            i.setType(types.length == 1 ? types[0] : "*/*");
            if (types.length > 1) i.putExtra(Intent.EXTRA_MIME_TYPES, types);
            try {
                startActivityForResult(i, PICK);
            } catch (Exception e) {
                toast("No file picker available");
                nativeOnPicked("", "");
            }
        });
    }

    public void saveToGallery(final String path, final String name, final String mime, final String album) {
        new Thread(() -> {
            try {
                boolean video = mime.startsWith("video/");
                boolean audio = mime.startsWith("audio/");
                Uri collection;
                String dir;
                if (video) {
                    collection = MediaStore.Video.Media.EXTERNAL_CONTENT_URI;
                    dir = Environment.DIRECTORY_MOVIES;
                } else if (audio) {
                    collection = MediaStore.Audio.Media.EXTERNAL_CONTENT_URI;
                    dir = Environment.DIRECTORY_MUSIC;
                } else {
                    collection = MediaStore.Images.Media.EXTERNAL_CONTENT_URI;
                    dir = Environment.DIRECTORY_PICTURES;
                }
                ContentValues v = new ContentValues();
                v.put(MediaStore.MediaColumns.DISPLAY_NAME, name);
                v.put(MediaStore.MediaColumns.MIME_TYPE, mime);
                String where;
                if (Build.VERSION.SDK_INT >= 29) {
                    v.put(MediaStore.MediaColumns.RELATIVE_PATH, dir + "/" + album);
                    v.put(MediaStore.MediaColumns.IS_PENDING, 1);
                    where = dir + "/" + album;
                } else {
                    File d = new File(Environment.getExternalStoragePublicDirectory(dir), album);
                    d.mkdirs();
                    v.put(MediaStore.MediaColumns.DATA, new File(d, name).getAbsolutePath());
                    where = d.getAbsolutePath();
                }
                ContentResolver cr = getContentResolver();
                Uri item = cr.insert(collection, v);
                if (item == null) throw new Exception("the gallery refused the file");
                try (InputStream in = new FileInputStream(path); OutputStream out = cr.openOutputStream(item)) {
                    copy(in, out);
                }
                if (Build.VERSION.SDK_INT >= 29) {
                    ContentValues done = new ContentValues();
                    done.put(MediaStore.MediaColumns.IS_PENDING, 0);
                    cr.update(item, done, null, null);
                }
                new File(path).delete();
                nativeOnSaved(true, "Saved to " + where + "/" + name);
            } catch (Exception e) {
                nativeOnSaved(false, "Couldn't save: " + e.getMessage());
            }
        }).start();
    }

    public void promptText(final String id, final String title, final String initial) {
        runOnUiThread(() -> {
            final android.widget.EditText input = new android.widget.EditText(this);
            input.setText(initial);
            input.setSelectAllOnFocus(true);
            input.setSingleLine(false);
            input.setMaxLines(4);
            android.app.AlertDialog d = new android.app.AlertDialog.Builder(this)
                    .setTitle(title)
                    .setView(input)
                    .setPositiveButton(android.R.string.ok, (dlg, w) -> nativeOnText(id, input.getText().toString()))
                    .setNegativeButton(android.R.string.cancel, null)
                    .create();
            d.setOnShowListener(x -> {
                input.requestFocus();
                d.getWindow().setSoftInputMode(android.view.WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE);
            });
            d.show();
        });
    }

    public void toast(final String message) {
        runOnUiThread(() -> Toast.makeText(this, message, Toast.LENGTH_SHORT).show());
    }

    // ---- picker result ---------------------------------------------------------------------

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != PICK) return;
        if (resultCode != RESULT_OK || data == null || data.getData() == null) {
            nativeOnPicked("", "");
            return;
        }
        final Uri uri = data.getData();
        // copy off the UI thread: videos can be large
        new Thread(() -> {
            String name = displayName(uri);
            File dir = new File(getCacheDir(), "picked");
            dir.mkdirs();
            File out = new File(dir, System.currentTimeMillis() + "-" + name.replaceAll("[/\\\\]", "_"));
            try (InputStream in = getContentResolver().openInputStream(uri); OutputStream o = new FileOutputStream(out)) {
                if (in == null) throw new Exception("unreadable");
                copy(in, o);
                nativeOnPicked(out.getAbsolutePath(), name);
            } catch (Exception e) {
                toast("Couldn't open the file: " + e.getMessage());
                nativeOnPicked("", "");
            }
        }).start();
    }

    private String displayName(Uri uri) {
        String name = "file";
        try (Cursor c = getContentResolver().query(uri, new String[]{OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst()) {
                String n = c.getString(0);
                if (n != null && !n.isEmpty()) name = n;
            }
        } catch (Exception ignored) {
        }
        return name;
    }

    private static void copy(InputStream in, OutputStream out) throws java.io.IOException {
        byte[] buf = new byte[1 << 16];
        int n;
        while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
    }
}

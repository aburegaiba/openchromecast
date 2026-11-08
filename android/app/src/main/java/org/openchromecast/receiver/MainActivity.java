package org.openchromecast.receiver;

import android.Manifest;
import android.content.Intent;
import android.os.*;
import android.view.*;
import android.widget.*;
import androidx.appcompat.app.AppCompatActivity;
import com.google.android.material.appbar.MaterialToolbar;
import com.google.android.material.button.MaterialButton;
import com.google.android.material.textfield.*;

public final class MainActivity extends AppCompatActivity {
    static boolean visible;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private TextView state;
    private MaterialButton start, stop;
    private TextInputEditText name;
    private final Runnable refresh = new Runnable() {
        @Override public void run() {
            state.setText(ReceiverService.summary);
            boolean running = ReceiverService.current != null;
            boolean nativeBusy = NativeBridge.running();
            start.setEnabled(!running && !nativeBusy); stop.setEnabled(running); name.setEnabled(!running && !nativeBusy);
            handler.postDelayed(this, 500);
        }
    };

    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        LinearLayout root = new LinearLayout(this); root.setOrientation(LinearLayout.VERTICAL);
        root.setFitsSystemWindows(true);
        MaterialToolbar toolbar = new MaterialToolbar(this); toolbar.setTitle("OpenChromecast");
        root.addView(toolbar, new LinearLayout.LayoutParams(-1, dp(64)));
        ScrollView scroll = new ScrollView(this);
        LinearLayout panel = new LinearLayout(this); panel.setOrientation(LinearLayout.VERTICAL);
        panel.setPadding(dp(24), dp(16), dp(24), dp(24));
        state = new TextView(this); state.setTextAppearance(com.google.android.material.R.style.TextAppearance_Material3_TitleLarge);
        panel.addView(state, spaced());
        TextInputLayout field = new TextInputLayout(this); field.setHint("Nom du recepteur");
        name = new TextInputEditText(field.getContext()); name.setSingleLine(true);
        name.setFilters(new android.text.InputFilter[]{new android.text.InputFilter.LengthFilter(48)});
        name.setText(getSharedPreferences("receiver", MODE_PRIVATE).getString("name", "OpenChromecast Android"));
        field.addView(name); panel.addView(field, spaced());
        start = button("Demarrer", android.R.drawable.ic_media_play);
        start.setOnClickListener(view -> {
            String value = name.getText() == null ? "" : name.getText().toString().trim();
            if (!MediaPolicy.acceptsName(value)) { field.setError("Nom vide, trop long ou invalide"); return; }
            field.setError(null);
            getSharedPreferences("receiver", MODE_PRIVATE).edit().putString("name", value).apply();
            if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED)
                requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 1);
            startForegroundService(new Intent(this, ReceiverService.class));
        });
        panel.addView(start, spaced());
        stop = button("Arreter", android.R.drawable.ic_media_pause);
        stop.setOnClickListener(view -> stopService(new Intent(this, ReceiverService.class)));
        panel.addView(stop, spaced());
        MaterialButton video = button("Ouvrir la video", android.R.drawable.ic_menu_view);
        video.setOnClickListener(view -> startActivity(new Intent(this, VideoActivity.class)));
        panel.addView(video, spaced());
        MaterialButton home = button("Home Assistant", android.R.drawable.ic_menu_revert);
        home.setOnClickListener(view -> {
            Intent companion = getPackageManager().getLaunchIntentForPackage("io.homeassistant.companion.android");
            if (companion != null) startActivity(companion); else finish();
        });
        panel.addView(home, spaced());
        scroll.addView(panel); root.addView(scroll, new LinearLayout.LayoutParams(-1, -1)); setContentView(root);
    }
    private MaterialButton button(String text, int icon) {
        MaterialButton button = new MaterialButton(this);
        button.setText(text); button.setIconResource(icon); button.setMinHeight(dp(48));
        return button;
    }
    private LinearLayout.LayoutParams spaced() {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2);
        params.bottomMargin = dp(12); return params;
    }
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }
    @Override protected void onResume() { super.onResume(); visible = true; handler.post(refresh); }
    @Override protected void onPause() { visible = false; handler.removeCallbacks(refresh); super.onPause(); }
}

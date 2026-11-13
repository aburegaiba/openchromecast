package org.openchromecast.receiver;

import android.annotation.SuppressLint;
import android.app.*;
import android.content.*;
import android.content.pm.ActivityInfo;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.content.res.Configuration;
import android.content.res.ColorStateList;
import android.graphics.PixelFormat;
import android.graphics.Rect;
import android.os.*;
import android.provider.Settings;
import android.view.*;
import android.widget.FrameLayout;
import android.widget.Toast;
import com.google.android.material.color.MaterialColors;
import com.google.android.material.floatingactionbutton.FloatingActionButton;

/** An opt-in launcher only: microphone access stays in Home Assistant Companion. */
public final class AssistOverlayService extends Service {
    private static final String CHANNEL = "assist_shortcut";
    private static final int NOTIFICATION = 4102;
    private static final String DISABLE = "org.openchromecast.receiver.HIDE_ASSIST_SHORTCUT";
    private static final long CLICK_INTERVAL_MS = 1500;
    static boolean running;
    private SharedPreferences preferences;
    private WindowManager windows;
    private FrameLayout overlay;
    private WindowManager.LayoutParams layout;
    private int left, right, top, bottom;
    private float downX, downY;
    private int originalX, originalY;
    private boolean dragging;
    private long lastLaunch;
    private final BroadcastReceiver screen = new BroadcastReceiver() {
        @Override public void onReceive(Context context, Intent intent) { refreshVisibility(); }
    };

    @Override public void onCreate() {
        super.onCreate();
        preferences = getSharedPreferences(AssistOverlayPolicy.PREFERENCES, MODE_PRIVATE);
        windows = getSystemService(WindowManager.class);
        IntentFilter filter = new IntentFilter();
        filter.addAction(Intent.ACTION_SCREEN_OFF);
        filter.addAction(Intent.ACTION_SCREEN_ON);
        filter.addAction(Intent.ACTION_USER_PRESENT);
        if (Build.VERSION.SDK_INT >= 33) registerReceiver(screen, filter, RECEIVER_NOT_EXPORTED);
        else registerReceiver(screen, filter);
    }

    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && DISABLE.equals(intent.getAction())) {
            preferences.edit().putBoolean(AssistOverlayPolicy.ENABLED, false).apply();
            stopSelf();
            return START_NOT_STICKY;
        }
        if (!preferences.getBoolean(AssistOverlayPolicy.ENABLED, false) || !Settings.canDrawOverlays(this)) {
            stopSelf();
            return START_NOT_STICKY;
        }
        try {
            Notification notification = notification();
            if (Build.VERSION.SDK_INT >= 34)
                startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
            else if (Build.VERSION.SDK_INT >= 29) startForeground(NOTIFICATION, notification, 0);
            else startForeground(NOTIFICATION, notification);
            running = true;
            refreshVisibility();
            return START_STICKY;
        } catch (IllegalStateException | SecurityException error) {
            preferences.edit().putBoolean(AssistOverlayPolicy.ENABLED, false).apply();
            stopSelf();
            return START_NOT_STICKY;
        }
    }

    private Notification notification() {
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel(CHANNEL,
            getString(R.string.assist_shortcut), NotificationManager.IMPORTANCE_LOW));
        int flags = PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT;
        PendingIntent settings = PendingIntent.getActivity(this, NOTIFICATION,
            new Intent(this, MainActivity.class), flags);
        PendingIntent hide = PendingIntent.getService(this, NOTIFICATION,
            new Intent(this, AssistOverlayService.class).setAction(DISABLE), flags);
        return new Notification.Builder(this, CHANNEL).setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentTitle(getString(R.string.assist_shortcut)).setContentText(getString(R.string.assist_shortcut_idle))
            .setContentIntent(settings).setOngoing(true).setOnlyAlertOnce(true).setShowWhen(false)
            .setCategory(Notification.CATEGORY_SERVICE)
            .addAction(new Notification.Action.Builder(android.R.drawable.ic_menu_close_clear_cancel,
                getString(R.string.hide_assist_shortcut), hide).build()).build();
    }

    @SuppressLint("RtlHardcoded") // Saved positions refer to physical screen edges, not text direction.
    private void refreshVisibility() {
        if (!running) return;
        PowerManager power = getSystemService(PowerManager.class);
        KeyguardManager keyguard = getSystemService(KeyguardManager.class);
        if (!power.isInteractive() || keyguard.isKeyguardLocked()) { removeOverlay(); return; }
        if (!Settings.canDrawOverlays(this)) {
            preferences.edit().putBoolean(AssistOverlayPolicy.ENABLED, false).apply();
            stopSelf();
            return;
        }
        if (overlay != null) return;
        Context theme = new ContextThemeWrapper(this, R.style.AppTheme);
        overlay = new FrameLayout(theme);
        overlay.setPadding(dp(4), dp(4), dp(4), dp(4));
        FloatingActionButton button = new ShortcutButton(theme);
        button.setCustomSize(dp(56));
        button.setImageResource(android.R.drawable.ic_btn_speak_now);
        button.setBackgroundTintList(ColorStateList.valueOf(MaterialColors.getColor(button,
            androidx.appcompat.R.attr.colorPrimary)));
        button.setImageTintList(ColorStateList.valueOf(MaterialColors.getColor(button,
            com.google.android.material.R.attr.colorOnPrimary)));
        button.setContentDescription(getString(R.string.open_assist));
        button.setTooltipText(getString(R.string.open_assist));
        button.setOnClickListener(view -> openAssist());
        overlay.addView(button, new FrameLayout.LayoutParams(dp(56), dp(56)));
        layout = new WindowManager.LayoutParams(dp(64), dp(64), WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE | WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT);
        layout.gravity = Gravity.TOP | Gravity.LEFT;
        positionFromPreferences();
        try { windows.addView(overlay, layout); }
        catch (IllegalStateException | SecurityException | WindowManager.BadTokenException error) {
            overlay = null;
            preferences.edit().putBoolean(AssistOverlayPolicy.ENABLED, false).apply();
            stopSelf();
        }
    }

    private final class ShortcutButton extends FloatingActionButton {
        ShortcutButton(Context context) { super(context); }
        @Override public boolean onTouchEvent(MotionEvent event) {
            if (layout == null) return false;
            switch (event.getActionMasked()) {
                case MotionEvent.ACTION_DOWN:
                    downX = event.getRawX(); downY = event.getRawY();
                    originalX = layout.x; originalY = layout.y; dragging = false;
                    setPressed(true);
                    return true;
                case MotionEvent.ACTION_MOVE:
                    float dx = event.getRawX() - downX, dy = event.getRawY() - downY;
                    if (!dragging && Math.hypot(dx, dy) > ViewConfiguration.get(getContext()).getScaledTouchSlop()) {
                        dragging = true; setPressed(false);
                    }
                    if (dragging) {
                        layout.x = AssistOverlayPolicy.clamp(originalX + Math.round(dx), left, right);
                        layout.y = AssistOverlayPolicy.clamp(originalY + Math.round(dy), top, bottom);
                        updateOverlay();
                    }
                    return true;
                case MotionEvent.ACTION_UP:
                    setPressed(false);
                    if (dragging) {
                        preferences.edit().putBoolean(AssistOverlayPolicy.RIGHT_EDGE, layout.x > left + (right - left) / 2)
                            .putFloat(AssistOverlayPolicy.VERTICAL_FRACTION,
                                AssistOverlayPolicy.verticalFraction(layout.y, top, bottom)).apply();
                        positionFromPreferences(); updateOverlay();
                    } else performClick();
                    return true;
                case MotionEvent.ACTION_CANCEL:
                    setPressed(false); positionFromPreferences(); updateOverlay(); return true;
                case MotionEvent.ACTION_POINTER_DOWN:
                    dragging = true; setPressed(false); return true;
                default: return true;
            }
        }
        @Override public boolean performClick() { return super.performClick(); }
    }

    private void positionFromPreferences() {
        Rect bounds = new Rect();
        int insetLeft = 0, insetTop = 0, insetRight = 0, insetBottom = 0;
        if (Build.VERSION.SDK_INT >= 30) {
            WindowMetrics metrics = windows.getCurrentWindowMetrics();
            bounds.set(metrics.getBounds());
            android.graphics.Insets insets = metrics.getWindowInsets().getInsetsIgnoringVisibility(
                WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
            insetLeft = insets.left; insetTop = insets.top; insetRight = insets.right; insetBottom = insets.bottom;
        } else windows.getDefaultDisplay().getRectSize(bounds);
        left = insetLeft + dp(8); top = insetTop + dp(8);
        right = Math.max(left, bounds.width() - insetRight - dp(72));
        bottom = Math.max(top, bounds.height() - insetBottom - dp(72));
        layout.x = preferences.getBoolean(AssistOverlayPolicy.RIGHT_EDGE, true) ? right : left;
        layout.y = AssistOverlayPolicy.verticalPosition(top, bottom,
            preferences.getFloat(AssistOverlayPolicy.VERTICAL_FRACTION, AssistOverlayPolicy.DEFAULT_FRACTION));
    }

    private void updateOverlay() {
        if (overlay == null) return;
        try { windows.updateViewLayout(overlay, layout); }
        catch (IllegalArgumentException | SecurityException error) { stopSelf(); }
    }

    private boolean available(String name) {
        try {
            ActivityInfo info = getPackageManager().getActivityInfo(new ComponentName(name,
                AssistOverlayPolicy.ACTIVITY), 0);
            return info.exported && info.enabled && info.applicationInfo.enabled;
        } catch (PackageManager.NameNotFoundException error) { return false; }
    }

    private void openAssist() {
        long now = SystemClock.elapsedRealtime();
        if (lastLaunch != 0 && now - lastLaunch < CLICK_INTERVAL_MS) return;
        lastLaunch = now;
        String target = AssistOverlayPolicy.companionPackage(available(AssistOverlayPolicy.FULL_PACKAGE),
            available(AssistOverlayPolicy.MINIMAL_PACKAGE));
        if (target == null) { Toast.makeText(this, R.string.assist_unavailable, Toast.LENGTH_LONG).show(); return; }
        Intent intent = new Intent(Intent.ACTION_ASSIST)
            .setComponent(new ComponentName(target, AssistOverlayPolicy.ACTIVITY))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            .putExtra("pipeline", "preferred").putExtra("start_listening", true).putExtra("from_frontend", false);
        try { startActivity(intent); }
        catch (ActivityNotFoundException | SecurityException error) {
            Toast.makeText(this, R.string.assist_unavailable, Toast.LENGTH_LONG).show();
        }
    }

    @Override public void onConfigurationChanged(Configuration next) {
        super.onConfigurationChanged(next);
        if (overlay != null) { positionFromPreferences(); updateOverlay(); }
    }

    private void removeOverlay() {
        if (overlay == null) return;
        try { windows.removeViewImmediate(overlay); } catch (IllegalArgumentException ignored) { }
        overlay = null;
    }

    @Override public void onDestroy() {
        running = false;
        removeOverlay();
        unregisterReceiver(screen);
        stopForeground(STOP_FOREGROUND_REMOVE);
        super.onDestroy();
    }

    @Override public IBinder onBind(Intent intent) { return null; }
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }
}

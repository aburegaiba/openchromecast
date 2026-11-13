package org.openchromecast.receiver;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.provider.Settings;
import android.util.Log;

public final class AssistOverlayBootReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context context, Intent intent) {
        boolean enabled = context.getSharedPreferences(AssistOverlayPolicy.PREFERENCES, Context.MODE_PRIVATE)
            .getBoolean(AssistOverlayPolicy.ENABLED, false);
        if (intent == null || !AssistOverlayPolicy.shouldResume(intent.getAction(), enabled,
                Settings.canDrawOverlays(context))) return;
        try {
            context.startForegroundService(new Intent(context, AssistOverlayService.class));
        } catch (IllegalStateException | SecurityException error) {
            Log.w("OpenChromecast", "Assist shortcut resume refused: " + error.getClass().getSimpleName());
        }
    }
}

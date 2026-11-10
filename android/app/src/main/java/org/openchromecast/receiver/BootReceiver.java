package org.openchromecast.receiver;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.util.Log;

public final class BootReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context context, Intent intent) {
        SharedPreferences settings = context.getSharedPreferences(ReceiverStartupPolicy.PREFERENCES, Context.MODE_PRIVATE);
        if (intent == null || !ReceiverStartupPolicy.shouldResume(intent.getAction(),
                settings.getBoolean(ReceiverStartupPolicy.AUTO_START, false),
                settings.getBoolean(ReceiverStartupPolicy.ENABLED, false))) return;
        try {
            // The service starts idle as connectedDevice, not mediaPlayback.
            context.startForegroundService(new Intent(context, ReceiverService.class));
        } catch (IllegalStateException | SecurityException error) {
            Log.w("OpenChromecast", "Automatic start refused: " + error.getClass().getSimpleName());
        }
    }
}

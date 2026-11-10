package org.openchromecast.receiver;

final class ReceiverStartupPolicy {
    static final String PREFERENCES = "receiver";
    static final String AUTO_START = "auto_start";
    static final String ENABLED = "enabled";

    private ReceiverStartupPolicy() {}

    static boolean shouldResume(String action, boolean automatic, boolean enabled) {
        return "android.intent.action.BOOT_COMPLETED".equals(action) && automatic && enabled;
    }
}

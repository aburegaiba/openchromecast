package org.openchromecast.receiver;

final class AssistOverlayPolicy {
    static final String PREFERENCES = "assist_overlay";
    static final String ENABLED = "enabled";
    static final String RIGHT_EDGE = "right_edge";
    static final String VERTICAL_FRACTION = "vertical_fraction";
    static final float DEFAULT_FRACTION = 0.45f;
    static final String FULL_PACKAGE = "io.homeassistant.companion.android";
    static final String MINIMAL_PACKAGE = "io.homeassistant.companion.android.minimal";
    static final String ACTIVITY = "io.homeassistant.companion.android.assist.AssistActivity";

    private AssistOverlayPolicy() {}

    static boolean shouldResume(String action, boolean enabled, boolean permission) {
        return enabled && permission && ("android.intent.action.BOOT_COMPLETED".equals(action)
            || "android.intent.action.MY_PACKAGE_REPLACED".equals(action));
    }

    static String companionPackage(boolean full, boolean minimal) {
        return full ? FULL_PACKAGE : minimal ? MINIMAL_PACKAGE : null;
    }

    static float fraction(float value) {
        return Float.isFinite(value) ? Math.max(0f, Math.min(1f, value)) : DEFAULT_FRACTION;
    }

    static int clamp(int value, int minimum, int maximum) {
        return Math.max(minimum, Math.min(Math.max(minimum, maximum), value));
    }

    static int verticalPosition(int top, int bottom, float value) {
        return top + Math.round(Math.max(0, bottom - top) * fraction(value));
    }

    static float verticalFraction(int position, int top, int bottom) {
        return bottom > top ? fraction((float) (position - top) / (bottom - top)) : DEFAULT_FRACTION;
    }
}

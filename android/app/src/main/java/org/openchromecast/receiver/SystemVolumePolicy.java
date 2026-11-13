package org.openchromecast.receiver;

final class SystemVolumePolicy {
    static final String MIGRATED = "system_volume_v1";

    static boolean ready(int minimum, int maximum) {
        return minimum >= 0 && maximum > minimum;
    }

    static int castIndex(float level, int minimum, int maximum) {
        if (!Float.isFinite(level)) throw new IllegalArgumentException("Invalid volume");
        return Math.round(minimum + Math.max(0f, Math.min(1f, level)) * range(minimum, maximum));
    }

    static int migrationIndex(int current, float legacyGain, int minimum, int maximum) {
        range(minimum, maximum);
        int bounded = Math.max(minimum, Math.min(maximum, current));
        return minimum + (int)Math.floor((bounded - minimum) * MediaPolicy.restoredVolume(legacyGain));
    }

    static float level(int index, int minimum, int maximum) {
        return Math.max(0f, Math.min(1f, (index - minimum) / (float)range(minimum, maximum)));
    }

    static float step(int minimum, int maximum) {
        return 1f / range(minimum, maximum);
    }

    private static int range(int minimum, int maximum) {
        if (minimum < 0 || maximum <= minimum) throw new IllegalArgumentException("Fixed device volume");
        return maximum - minimum;
    }

    private SystemVolumePolicy() {}
}

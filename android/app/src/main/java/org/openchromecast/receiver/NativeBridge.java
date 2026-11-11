package org.openchromecast.receiver;

final class NativeBridge {
    static { System.loadLibrary("openchromecast_android"); }
    static native String start(int port, String deviceId, String name, float initialVolume,
        String privateDirectory, String networkPrefixes);
    static native void networkChanged(String networkPrefixes);
    static native String poll();
    static native boolean running();
    static native void stop();
    static native void status(long generation, String state, float position,
        float duration, float volume, boolean muted, boolean error);
    private NativeBridge() {}
}

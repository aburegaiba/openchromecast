package org.openchromecast.receiver;

final class VideoPresentationPolicy {
    static final String AUTOMATIC_VIDEO = "automatic_video";

    private VideoPresentationPolicy() {}

    static boolean shouldPresent(boolean video, boolean alreadyRequested, boolean controlsVisible,
            boolean automaticEnabled, boolean backgroundPermissionGranted) {
        return video && !alreadyRequested
            && (controlsVisible || (automaticEnabled && backgroundPermissionGranted));
    }
}

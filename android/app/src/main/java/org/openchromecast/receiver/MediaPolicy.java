package org.openchromecast.receiver;

import java.net.URI;
import java.net.URISyntaxException;

final class MediaPolicy {
    static boolean acceptsManifest(String mime, String manifest) {
        return "application/dash+xml".equals(mime) && manifest != null && !manifest.isEmpty()
            && manifest.getBytes(java.nio.charset.StandardCharsets.UTF_8).length <= 1024 * 1024;
    }
    static boolean acceptsHeader(String name, String value) {
        return name != null && value != null && name.length() <= 128 && value.length() <= 8192
            && name.matches("[!#$%&'*+.^_`|~0-9A-Za-z-]+")
            && value.indexOf('\r') < 0 && value.indexOf('\n') < 0;
    }
    static boolean acceptsName(String name) {
        return name != null && !name.trim().isEmpty()
            && name.getBytes(java.nio.charset.StandardCharsets.UTF_8).length <= 63
            && name.chars().noneMatch(Character::isISOControl);
    }
    static boolean accepts(String url) {
        if (url == null || url.length() > 8192) return false;
        try {
            URI parsed = new URI(url);
            String scheme = parsed.getScheme();
            return ("http".equalsIgnoreCase(scheme) || "https".equalsIgnoreCase(scheme))
                && parsed.getHost() != null && parsed.getUserInfo() == null;
        } catch (URISyntaxException e) { return false; }
    }
    private MediaPolicy() {}
}

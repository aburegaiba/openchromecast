package org.openchromecast.receiver;

import java.net.URI;
import java.net.URISyntaxException;

final class MediaPolicy {
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

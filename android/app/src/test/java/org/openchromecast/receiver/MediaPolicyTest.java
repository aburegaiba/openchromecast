package org.openchromecast.receiver;
import org.junit.Test;
import static org.junit.Assert.*;

public class MediaPolicyTest {
    @Test public void validatesUtf8DiscoveryName() {
        assertTrue(MediaPolicy.acceptsName("Receiver"));
        assertFalse(MediaPolicy.acceptsName(" "));
        assertFalse(MediaPolicy.acceptsName("x\nreceiver"));
        assertFalse(MediaPolicy.acceptsName("\u00e9".repeat(32)));
        assertTrue(MediaPolicy.acceptsName("\u00e9".repeat(31)));
    }
    @Test public void acceptsLanAndTlsMedia() {
        assertTrue(MediaPolicy.accepts("http://192.0.2.1:8123/media/test.mp3"));
        assertTrue(MediaPolicy.accepts("https://example.test/test.mp4?token=example"));
    }
    @Test public void refusesFilesAndOtherSchemes() {
        for (String url : new String[]{"file:///data/data/test", "content://test", "data:audio/mp3,x", "ftp://example.test/a"})
            assertFalse(MediaPolicy.accepts(url));
    }
    @Test public void refusesMalformedAndEmbeddedCredentials() {
        assertFalse(MediaPolicy.accepts("https://user:password@example.test/a"));
        assertFalse(MediaPolicy.accepts("http://"));
        assertFalse(MediaPolicy.accepts(null));
        assertFalse(MediaPolicy.accepts("http://example.test/" + "x".repeat(8192)));
    }
}

package org.openchromecast.receiver;

import android.content.Context;
import android.util.Base64;
import androidx.media3.common.MediaItem;
import androidx.media3.datasource.DefaultDataSource;
import androidx.media3.datasource.DefaultHttpDataSource;
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory;
import androidx.media3.exoplayer.source.MediaSource;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.Iterator;
import java.util.Map;
import org.json.JSONException;
import org.json.JSONObject;

@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
final class NativeMediaSource {
    static MediaSource create(Context context, JSONObject event) throws JSONException {
        String uri;
        String mime = event.optString("mime_type", "");
        if (event.has("manifest")) {
            String manifest = event.getString("manifest");
            if (!MediaPolicy.acceptsManifest(mime, manifest)) throw new IllegalArgumentException("Invalid native manifest");
            uri = "data:application/dash+xml;base64," + Base64.encodeToString(manifest.getBytes(StandardCharsets.UTF_8), Base64.NO_WRAP);
        } else {
            uri = event.getString("url");
            if (!MediaPolicy.accepts(uri)) throw new IllegalArgumentException("Invalid media address");
        }
        Map<String, String> headers = new HashMap<>();
        JSONObject source = event.optJSONObject("request_headers");
        if (source != null) {
            if (source.length() > 32) throw new IllegalArgumentException("Too many headers");
            Iterator<String> keys = source.keys();
            while (keys.hasNext()) {
                String key = keys.next();
                String value = source.getString(key);
                if (!MediaPolicy.acceptsHeader(key, value)) throw new IllegalArgumentException("Invalid media header");
                headers.put(key, value);
            }
        }
        DefaultHttpDataSource.Factory http = new DefaultHttpDataSource.Factory().setDefaultRequestProperties(headers);
        DefaultDataSource.Factory sources = new DefaultDataSource.Factory(context, http);
        MediaItem.Builder item = new MediaItem.Builder().setUri(uri);
        if (!mime.isEmpty()) item.setMimeType(mime);
        return new DefaultMediaSourceFactory(sources).createMediaSource(item.build());
    }
    private NativeMediaSource() {}
}

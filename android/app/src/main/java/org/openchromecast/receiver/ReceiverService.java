package org.openchromecast.receiver;

import android.app.*;
import android.content.*;
import android.content.pm.ServiceInfo;
import android.media.session.MediaSession;
import android.media.session.PlaybackState;
import android.net.*;
import android.net.nsd.*;
import android.net.wifi.WifiManager;
import android.os.*;
import android.provider.Settings;
import androidx.media3.common.*;
import androidx.media3.exoplayer.ExoPlayer;
import org.json.*;
import java.util.UUID;
import java.io.File;

public final class ReceiverService extends Service {
    private static final String CHANNEL = "cast_receiver";
    private static final int PORT = 8009;
    static ReceiverService current;
    static String summary = "Arrete";
    final Handler handler = new Handler(Looper.getMainLooper());
    ExoPlayer player;
    boolean video;
    private boolean destroying, ready, registered, muted, playbackError, videoPresentationRequested;
    private float volume = 0.15f;
    private long generation;
    private NsdManager nsd;
    private NsdManager.RegistrationListener registration;
    private WifiManager.MulticastLock multicast;
    private ConnectivityManager connectivity;
    private ConnectivityManager.NetworkCallback networkCallback;
    private final NetworkSelection networkSelection = new NetworkSelection();
    private MediaSession mediaSession;
    private String deviceId, receiverName;

    @Override public void onCreate() {
        super.onCreate();
        current = this;
        summary = "Demarrage...";
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel(CHANNEL, "Reception Cast", NotificationManager.IMPORTANCE_LOW));
        foreground(false);
        SharedPreferences settings = getSharedPreferences("receiver", MODE_PRIVATE);
        volume = MediaPolicy.restoredVolume(settings.getFloat("volume", 0.15f));
        muted = settings.getBoolean("muted", false);
        deviceId = settings.getString("device_id", null);
        if (deviceId == null) {
            deviceId = UUID.randomUUID().toString().replace("-", "");
            if (!settings.edit().putString("device_id", deviceId).commit()) { fail("Identite non sauvegardee"); return; }
        }
        receiverName = settings.getString("name", "OpenChromecast Android");
        if (!MediaPolicy.acceptsName(receiverName)) { fail("Nom de recepteur invalide"); return; }
        player = new ExoPlayer.Builder(this).build();
        player.setAudioAttributes(new AudioAttributes.Builder().setUsage(C.USAGE_MEDIA)
            .setContentType(C.AUDIO_CONTENT_TYPE_MOVIE).build(), true);
        player.setHandleAudioBecomingNoisy(true);
        player.setWakeMode(C.WAKE_MODE_NETWORK);
        player.setVolume(muted ? 0f : volume);
        player.addListener(new Player.Listener() {
            @Override public void onPlayerError(PlaybackException error) {
                playbackError = true;
                summary = "Lecture impossible (" + error.getErrorCodeName() + ")";
                VideoActivity.closeIfOpen();
                notifyState();
                publish();
            }
            @Override public void onPlaybackStateChanged(int state) {
                publish();
                if (state == Player.STATE_ENDED) VideoActivity.closeIfOpen();
            }
            @Override public void onIsPlayingChanged(boolean playing) { publish(); }
            @Override public void onTracksChanged(Tracks tracks) {
                // HLS/DASH content types need not start with video/.
                if (tracks.isTypeSelected(C.TRACK_TYPE_VIDEO) && !video) {
                    video = true; summary = "Video recue"; notifyState();
                    presentVideoIfAllowed();
                }
            }
        });
        mediaSession = new MediaSession(this, "OpenChromecast");
        mediaSession.setCallback(new MediaSession.Callback() {
            @Override public void onPlay() { player.play(); }
            @Override public void onPause() { player.pause(); }
            @Override public void onSeekTo(long position) { player.seekTo(position); }
            @Override public void onStop() { stopMedia(); }
        }, handler);
        mediaSession.setActive(true);
        nsd = getSystemService(NsdManager.class);
        WifiManager wifi = (WifiManager)getApplicationContext().getSystemService(WIFI_SERVICE);
        multicast = wifi.createMulticastLock("openchromecast-discovery");
        multicast.setReferenceCounted(false);
        multicast.acquire();
        connectivity = getSystemService(ConnectivityManager.class);
        networkSelection.available(connectivity.getActiveNetwork());
        networkCallback = new ConnectivityManager.NetworkCallback() {
            @Override public void onAvailable(Network network) {
                handler.post(() -> {
                    if (!destroying) {
                        networkSelection.available(network);
                        NativeBridge.networkChanged("[]");
                    }
                    if (ready && !destroying) {
                        withdraw();
                        handler.postDelayed(() -> { if (ready && !destroying) advertise(); }, 500);
                    }
                });
            }
            @Override public void onLost(Network network) {
                handler.post(() -> {
                    if (!destroying && networkSelection.lost(network)) {
                        NativeBridge.networkChanged("[]");
                        withdraw(); summary = "Reseau deconnecte"; notifyState();
                        handler.postDelayed(() -> { if (ready && !destroying) advertise(); }, 500);
                    }
                });
            }
            @Override public void onLinkPropertiesChanged(Network network, LinkProperties properties) {
                handler.post(() -> {
                    if (!destroying && networkSelection.isCurrent(network)) {
                        NativeBridge.networkChanged(networkPrefixes(properties).toString());
                    }
                });
            }
        };
        connectivity.registerDefaultNetworkCallback(networkCallback);
        try {
            File privateDirectory = getNoBackupFilesDir();
            android.system.Os.chmod(privateDirectory.getAbsolutePath(), 0700);
            File youtube = new File(privateDirectory, "youtube");
            if (!youtube.isDirectory() && !youtube.mkdir()) { fail("Dossier YouTube indisponible"); return; }
            android.system.Os.chmod(youtube.getAbsolutePath(), 0700);
            String error = NativeBridge.start(PORT, deviceId, receiverName, volume,
                privateDirectory.getAbsolutePath(), currentNetworkPrefixes().toString());
            if (!error.isEmpty()) { fail(error); return; }
        } catch (android.system.ErrnoException error) { fail("Dossier YouTube non protege"); return; }
        catch (LinkageError error) { fail("Bibliotheque native incompatible"); return; }
        handler.post(pump);
    }

    private JSONArray currentNetworkPrefixes() {
        Network active = connectivity.getActiveNetwork();
        return networkPrefixes(active == null ? null : connectivity.getLinkProperties(active));
    }

    private JSONArray networkPrefixes(LinkProperties properties) {
        JSONArray prefixes = new JSONArray();
        if (properties != null) {
            for (LinkAddress link : properties.getLinkAddresses()) {
                String prefix = NetworkPrefixes.format(link.getAddress(), link.getPrefixLength());
                if (prefix != null) prefixes.put(prefix);
            }
        }
        return prefixes;
    }

    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        SharedPreferences settings = getSharedPreferences(ReceiverStartupPolicy.PREFERENCES, MODE_PRIVATE);
        if (intent != null && "stop".equals(intent.getAction())) {
            settings.edit().putBoolean(ReceiverStartupPolicy.ENABLED, false).apply();
            stopSelf();
            return START_NOT_STICKY;
        }
        return settings.getBoolean(ReceiverStartupPolicy.AUTO_START, false)
            && settings.getBoolean(ReceiverStartupPolicy.ENABLED, false) ? START_STICKY : START_NOT_STICKY;
    }

    private final Runnable pump = new Runnable() {
        @Override public void run() {
            if (destroying) return;
            try {
                JSONArray events = new JSONArray(NativeBridge.poll());
                for (int i = 0; i < events.length(); i++) command(events.getJSONObject(i));
                if (!destroying) publish();
            } catch (JSONException error) { fail("Reponse native invalide"); }
            if (!destroying) handler.postDelayed(this, 250);
        }
    };

    @androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
    private void command(JSONObject event) throws JSONException {
        if (destroying) return;
        String kind = event.getString("type");
        if ("ready".equals(kind)) { ready = true; advertise(); return; }
        if ("error".equals(kind)) { fail(event.optString("message", "Erreur du recepteur")); return; }
        if ("stopped".equals(kind)) { if (!destroying) fail("Recepteur interrompu"); return; }
        generation = event.optLong("generation", generation);
        switch (kind) {
            case "load":
                try {
                    player.setMediaSource(NativeMediaSource.create(this, event));
                } catch (IllegalArgumentException error) {
                    stopMedia(); playbackError = true;
                    summary = "Source multimedia refusee"; notifyState(); publish(); return;
                }
                playbackError = false;
                videoPresentationRequested = false;
                video = event.optBoolean("video", false);
                player.seekTo(Math.max(0, (long)(event.optDouble("position", 0) * 1000)));
                player.setPlayWhenReady(event.optBoolean("autoplay", true));
                player.prepare();
                summary = video ? "Video recue" : "Audio recu";
                foreground(true);
                if (video) presentVideoIfAllowed();
                else VideoActivity.closeIfOpen();
                notifyState();
                break;
            case "play": player.play(); break;
            case "pause": player.pause(); break;
            case "seek": player.seekTo(Math.max(0, (long)(event.optDouble("position", 0) * 1000))); break;
            case "stop": stopMedia(); break;
            case "playback_error":
                stopMedia(); playbackError = true;
                summary = "Source multimedia non prise en charge"; notifyState(); publish(); break;
            case "volume": volume = (float)Math.max(0, Math.min(1, event.optDouble("volume", volume))); applyVolume(); break;
            case "mute": muted = event.optBoolean("muted", false); applyVolume(); break;
            default: break;
        }
    }

    void stopMedia() {
        player.stop(); player.clearMediaItems(); video = false;
        videoPresentationRequested = false;
        VideoActivity.closeIfOpen();
        summary = "Pret a recevoir";
        foreground(false);
        publish(); notifyState();
    }

    private void presentVideoIfAllowed() {
        boolean automatic = getSharedPreferences(ReceiverStartupPolicy.PREFERENCES, MODE_PRIVATE)
            .getBoolean(VideoPresentationPolicy.AUTOMATIC_VIDEO, false);
        if (!VideoPresentationPolicy.shouldPresent(video, videoPresentationRequested,
                MainActivity.visible, automatic, Settings.canDrawOverlays(this))) return;
        videoPresentationRequested = true;
        if (VideoActivity.isOpen()) return;
        try {
            // A separate task preserves the dashboard task beneath the player.
            startActivity(new Intent(this, VideoActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        } catch (android.content.ActivityNotFoundException | SecurityException ignored) {
            notifyState();
        }
    }

    private void applyVolume() {
        getSharedPreferences("receiver", MODE_PRIVATE).edit()
            .putFloat("volume", volume).putBoolean("muted", muted).apply();
        player.setVolume(muted ? 0f : volume); publish();
    }

    private void publish() {
        if (destroying || player == null) return;
        String state;
        int playback = player.getPlaybackState();
        if (playbackError || playback == Player.STATE_ENDED) state = "ENDED";
        else if (playback == Player.STATE_BUFFERING) state = "BUFFERING";
        else if (playback == Player.STATE_READY) state = player.getPlayWhenReady() ? "PLAYING" : "PAUSED";
        else state = "IDLE";
        long duration = player.getDuration();
        NativeBridge.status(generation, state, player.getCurrentPosition() / 1000f,
            duration == C.TIME_UNSET ? 0 : duration / 1000f, volume, muted, playbackError);
        if (mediaSession != null) {
            int androidState = player.isPlaying() ? PlaybackState.STATE_PLAYING : PlaybackState.STATE_PAUSED;
            mediaSession.setPlaybackState(new PlaybackState.Builder()
                .setActions(PlaybackState.ACTION_PLAY | PlaybackState.ACTION_PAUSE | PlaybackState.ACTION_STOP | PlaybackState.ACTION_SEEK_TO)
                .setState(androidState, player.getCurrentPosition(), player.isPlaying() ? 1 : 0).build());
        }
    }

    private void advertise() {
        if (destroying || registered || registration != null) return;
        if (connectivity.getActiveNetwork() == null) return;
        NsdServiceInfo info = new NsdServiceInfo();
        info.setServiceName(receiverName);
        info.setServiceType(CastDiscoveryPolicy.REGISTRATION_TYPE);
        info.setPort(PORT);
        info.setAttribute("id", deviceId); info.setAttribute("fn", receiverName);
        info.setAttribute("md", "OpenChromecast Android"); info.setAttribute("ve", "05");
        info.setAttribute("ca", Integer.toString(CastDiscoveryPolicy.CAPABILITIES)); info.setAttribute("st", "0");
        info.setAttribute("nf", "1"); info.setAttribute("rs", "");
        registration = new NsdManager.RegistrationListener() {
            @Override public void onServiceRegistered(NsdServiceInfo service) {
                handler.post(() -> { if (!destroying && registration == this) { registered = true; summary = "Pret a recevoir"; notifyState(); } });
            }
            @Override public void onRegistrationFailed(NsdServiceInfo service, int code) {
                handler.post(() -> { if (registration == this) fail("Decouverte indisponible (" + code + ")"); });
            }
            @Override public void onServiceUnregistered(NsdServiceInfo service) {}
            @Override public void onUnregistrationFailed(NsdServiceInfo service, int code) {}
        };
        nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, registration);
    }

    private void withdraw() {
        if (registration != null) {
            try { nsd.unregisterService(registration); } catch (IllegalArgumentException ignored) {}
            registration = null; registered = false;
        }
    }

    private Notification notification() {
        Intent open = new Intent(this, video ? VideoActivity.class : MainActivity.class);
        PendingIntent content = PendingIntent.getActivity(this, video ? 2 : 1, open, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        PendingIntent stop = PendingIntent.getService(this, 3, new Intent(this, ReceiverService.class).setAction("stop"), PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        return new Notification.Builder(this, CHANNEL).setSmallIcon(android.R.drawable.ic_media_play)
            .setContentTitle("OpenChromecast").setContentText(summary)
            .setOngoing(true).setContentIntent(content)
            .addAction(new Notification.Action.Builder(android.R.drawable.ic_menu_close_clear_cancel, "Arreter", stop).build()).build();
    }

    private void foreground(boolean playing) {
        if (Build.VERSION.SDK_INT >= 29) {
            int type = ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE;
            if (playing) type |= ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK;
            startForeground(1, notification(), type);
        } else startForeground(1, notification());
    }

    private void notifyState() { getSystemService(NotificationManager.class).notify(1, notification()); }
    private void fail(String reason) { summary = reason; stopSelf(); }

    @Override public void onDestroy() {
        destroying = true;
        handler.removeCallbacksAndMessages(null);
        withdraw();
        if (connectivity != null && networkCallback != null) connectivity.unregisterNetworkCallback(networkCallback);
        if (multicast != null && multicast.isHeld()) multicast.release();
        try { NativeBridge.stop(); } catch (LinkageError ignored) {}
        if (player != null) player.release();
        if (mediaSession != null) mediaSession.release();
        VideoActivity.closeIfOpen();
        current = null;
        if ("Pret a recevoir".equals(summary) || "Audio recu".equals(summary) || "Video recue".equals(summary)) summary = "Arrete";
        stopForeground(STOP_FOREGROUND_REMOVE);
        super.onDestroy();
    }
    @Override public IBinder onBind(Intent intent) { return null; }
}

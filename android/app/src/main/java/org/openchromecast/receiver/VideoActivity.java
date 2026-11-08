package org.openchromecast.receiver;

import android.os.Bundle;
import android.view.WindowManager;
import android.widget.TextView;
import androidx.appcompat.app.AppCompatActivity;
import androidx.media3.ui.PlayerView;

public final class VideoActivity extends AppCompatActivity {
    private static VideoActivity opened;
    private PlayerView view;
    @Override public void onCreate(Bundle state) {
        super.onCreate(state); opened = this;
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        ReceiverService service = ReceiverService.current;
        if (service == null || !service.video) {
            TextView empty = new TextView(this); empty.setText(R.string.no_video);
            empty.setGravity(android.view.Gravity.CENTER); setContentView(empty); return;
        }
        view = new PlayerView(this); view.setPlayer(service.player); setContentView(view);
    }
    static void closeIfOpen() { if (opened != null) opened.finish(); }
    @Override protected void onDestroy() {
        if (view != null) view.setPlayer(null);
        if (opened == this) opened = null;
        super.onDestroy();
    }
}

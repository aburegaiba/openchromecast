# Android control surface

Native operational settings, not a media catalogue or replacement dashboard.
Material 3 day/night theme, standard system typography and Back behavior.
Single scrollable vertical flow: state, receiver name, automatic-start and
optional automatic-video switches, start/stop, video, return.
Use platform media/action icons and 48dp minimum targets with 12dp separation.
System bars remain outside the content. No nested cards or custom navigation.
Real startup/failure/running states determine enabled controls; no fake metrics.
The video player is full-size in its own task and leaves the previous
application intact. Automatic presentation requires explicit opt-in and the
native Android permission screen; no overlay window is drawn. STOP, end and
player error close that video task instead of launching another application.

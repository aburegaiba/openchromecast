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

The optional Assist shortcut is a separate, user-enabled floating action
button: native microphone icon, 56dp target, theme primary/on-primary colors,
draggable with edge snapping and a saved relative height. It remains available
independently of Cast, does not take focus and hides while locked or screen-off.
It opens the installed Home Assistant Assist task only on a tap; microphone
state and conversation feedback belong to Companion, not this receiver.
The receiver settings and a quiet notification can disable the shortcut.

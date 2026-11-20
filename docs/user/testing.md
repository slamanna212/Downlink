# Testing Downlink

Thank you for testing. This page says what to check, what to expect, and how to send a useful report.
Everything here is pre-release: expect rough edges, and tell us about them.

## Before you start

- Install a dev build from the Releases page (see the README for the first-launch steps on your system).
- Add your Dispatcharr source (server address and API key). Downlink only ever talks to that server.
- Note your machine: OS and version, GPU, display setup (monitors, scaling), and for Linux the desktop
  (GNOME/KDE) and session type (Wayland/X11).

## What to check

Tick what works, and note anything odd. A phone photo or screen recording of problems is welcome (no real channel
artwork in public places, please).

### 1. Video under the interface (most important)

- Play a channel. The picture fills the player area and the controls bar is drawn over it.
- The controls bar is translucent and the video is visible through it. Buttons respond to clicks.
- Resize the window and move it between monitors (if you have two). The picture follows without flicker.
- Smoothness: motion looks even, no tearing, no stutter (sports or a scrolling ticker show it best).

### 2. Fullscreen

- Press **F11** to enter fullscreen and **Esc** to leave it. Esc must always leave fullscreen.
- In fullscreen the video stays under the controls and clicks still work.
- macOS: switch Spaces (swipe) and come back while in fullscreen: video must still be there, below the controls.
- Windows: with the taskbar set to auto-hide, the video must not jump or show a gap.

### 3. Zap feel

- Change channels ten times in a row. First picture should appear in about a second on a normal connection
  (longer if your server has to start the channel first).
- Browse the guide and the list for a minute without playing: nothing should start playing or buffering.
- Pick a channel that fails (stopped, or wrong source): you should see a plain message and a Retry button, not a hang.

### 4. Catch-up (if your source has an archive)

- Play a past programme from the guide. Seek forward and back. Pause and resume.
- Go live again. Switch channels while watching catch-up and check the server shows no leftover session.

### 5. Media keys and OS controls

- Play/pause media key pauses and resumes. The OS media widget shows what is playing
  (Linux: the GNOME/KDE media control; Windows: the volume flyout; macOS: Control Centre).
- Tray icon (Windows/macOS, Linux with AppIndicator): Show/hide, Play/pause, Quit work.

### 6. Sleep and lid close

- Start playing, then suspend the machine (or close the lid) and wake it. Playback must be paused, not running in
  the background. On wake, press play: it recovers.
- While playing, the screen does not dim or lock by itself.

### 7. Updater

- Settings, Updates: choose the **dev** channel for pre-release builds. **Check now** reports the result.
- When a newer build exists it downloads in the background and installs when you quit; playback is never
  interrupted. Start the app again and check the version in Settings.

### 8. Offline

- Disconnect the network (or stop the server). The app still opens with your channels and guide and shows an
  offline notice. Playing a channel gives a clear message.

## Sending a bug report

1. In Settings → About & diagnostics, choose **Export diagnostics**. Downlink writes a zip into your Downloads folder and
   offers to show it. The bundle holds redacted logs, app and server versions, player state, the hardware decoder in
   use, your OS and GPU details, and the last 50 player state changes. Credentials and stream addresses are scrubbed.
2. Open the zip and look through it if you like; it is plain text and JSON.
3. Send the zip with a short description: what you did, what you expected, what happened, and the machine details
   from "Before you start".

For a crash, also send the time it happened. For picture problems ("green screen", wrong colours) the
hardware-decoder line in the bundle is the first thing we look at; there is also a **Hardware decoding** setting in Settings,
Playback, which is worth trying and reporting.

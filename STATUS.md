# Status

The current state of Laser Level Webcam 2.x. Read the known limits before you report a
problem: a request to remove a limit is a feature request, not a bug.

## What works

- The four steps: **Set up**, **Wait**, **Measure** and **Results**, in the window app and in
  the browser version.
- Live cameras on Windows (Media Foundation), Linux (Video4Linux) and macOS (AVFoundation), and
  in the browser.
- The Synthetic camera, which needs no hardware. It makes a beam on a simulated surface, so
  you can try each step without a camera and a laser.
- Saved sensors, sessions, and settings that come back when you start the app again.
- The command-line tool `laser`. It writes JSON.

## What was checked

- `cargo clippy` and `cargo test` on Windows, Linux and macOS, in GitHub Actions, for each push.
- The Windows app with live USB cameras.
- The browser version in Chrome on Windows, with `browser-check` and `pack check`.
- On Apple Silicon with macOS 27.0.1, the MacBook Pro Camera and Logitech C922 supplied frames
  through the `laser` tool. Six modes on each camera passed frame-size and timing checks. A
  separate C922 **640×360, 24 fps, NV12** check delivered about 24 fps after output negotiation
  was moved after source-format selection and made to prefer supported full-range NV12.
- The C922 passed **Exposure** and **Focus** auto/lock readback, reopening and refusal of numeric
  values. The MacBook reported no auto/lock controls. Frame gaps were observed; these checks do
  not establish measurement accuracy or gap-free capture at every rate.
- Linux live cameras, Intel Macs and older macOS versions have not been checked here.

## Known limits

- The Mac app sets no camera values, because macOS gives the app none. **Exposure**, **White
  balance** and **Focus** are either **Auto** or locked where they are, if the camera allows both.
  The Mac app has no gain and no other controls. White-balance locking was not available on the
  cameras checked.
- The Mac app and the browser version do not get the USB descriptors of the camera. Thus
  **What the camera reports** is not available, and a saved sensor is found by the name of the
  camera.
- The browser version gets colour frames that the browser has processed, not the YUYV or
  MJPEG data. The camera controls are the ones that the browser gives.
- The Linux app is for 64-bit x86 only, with glibc 2.35 or newer. The Mac app needs macOS 11 or
  newer.
- The Windows, Mac and Linux files have no code signature. Windows and macOS show a warning the
  first time that you start them.
- The LinuxCNC remote driver of 1.x is not in 2.x. It is on the `python` branch.

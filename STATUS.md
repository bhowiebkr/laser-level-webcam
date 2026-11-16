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
- The Windows app opened its window and drew with each renderer: Direct3D 12, Direct3D 12 on
  the processor (WARP), Vulkan and OpenGL. This was on one computer with an NVIDIA graphics
  card. A virtual machine and a Remote Desktop session have not been tried.
- The browser version in Chrome on Windows, with `browser-check` and `pack check`.
- The `laser` tool on a Mac with Apple silicon and macOS 27, with the MacBook Pro camera and a
  Logitech C922 (reported in
  [pull request 47](https://github.com/bhowiebkr/laser-level-webcam/pull/47)):
  - Six modes on each camera gave frames of the correct size, at no more than 15% above the
    frame rate of the mode.
  - The C922 at **640×360, 24 fps, NV12** gave about 24 fps. Before that pull request, it gave
    about 30 fps.
  - On the C922, **Auto** and lock operated for **Exposure** and **Focus**, also after the camera
    was opened again. The MacBook Pro camera gives no controls.
  - Some frames were missing. These checks do not show the accuracy of a measurement.
- The Mac window app and the Linux app have not been tried with live cameras. Intel Macs and
  older versions of macOS have not been tried.

## Known limits

- The Mac app sets no camera values, because macOS gives the app none. **Exposure**, **White
  balance** and **Focus** are either **Auto** or locked where they are, if the camera allows both.
  The Mac app has no gain and no other controls. On the cameras that were tried, **White
  balance** could not be locked.
- The Mac app and the browser version do not get the USB descriptors of the camera. Thus
  **What the camera reports** is not available, and a saved sensor is found by the name of the
  camera.
- The browser version gets colour frames that the browser has processed, not the YUYV or
  MJPEG data. The camera controls are the ones that the browser gives.
- The browser version keeps the session in the storage of the browser, which holds about 5 MB
  for a page. A browser can refuse a long session. Then the red chip **"Browser did not keep the
  session"** shows, and **Save session…** keeps the session in a file.
- A run keeps the camera mode and the line direction of its first sample. To measure with
  another mode or direction, click **New run**.
- The Linux app is for 64-bit x86 only, with glibc 2.35 or newer. The Mac app needs macOS 11 or
  newer.
- The Windows, Mac and Linux files have no code signature. Windows and macOS show a warning the
  first time that you start them.
- The LinuxCNC remote driver of 1.x is not in 2.x. It is on the `python` branch.

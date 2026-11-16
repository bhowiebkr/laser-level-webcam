# Changelog

What changed in each version. The [releases](https://github.com/bhowiebkr/laser-level-webcam/releases)
have the files. The [README](README.md#getting-started) tells how to install them.

## Not released yet

- Graphs: the y labels fit, a value that rounds to zero has no minus sign, the unit shows once
  and the Wait clock does not repeat a time.
- A run keeps the frame size and line direction of its first sample. A sample in another camera
  mode or orientation is refused.
- **Ready to measure** needs the drift rate inside the allowed rate by two standard errors, so a
  noisy beam no longer gives an early Ready.
- The **Scaled** sensor scale is right for a mode of another shape than the sensor.
- The step test of the drift monitor has the correct standard error.
- A long Wait no longer slows the view: the settle fit uses at most 900 points.
- A grid with all its stations in one row says that a flatness needs another row. **Add station**
  fills the last row of a grid first.
- **Use part of the frame** starts with the middle half along the line.
- The Synthetic camera opens with its beam not clipped.
- A camera opens without saved controls from another camera.
- The browser saves a large session as plain JSON, and a red chip says when the browser refuses
  the save.
- The desktop app writes `crash.log` on a crash or a start error. On Windows a message shows the
  error and the path of the log.
- A bad frame header gives an error, not a crash.
- A camera opens in MJPEG when MJPEG gives the full size at a higher frame rate.
- Hover texts on the controls, readings, graphs and diagrams.
- **Allowed drift** and the sample target show and take the selected unit.
- The README, CONTRIBUTING.md and AGENTS.md point to the
  [wiki](https://github.com/bhowiebkr/laser-level-webcam/wiki) for set-ups, cameras and results.
- Windows: when Direct3D 12 cannot open the window, the app tries Direct3D 12 on the processor
  (WARP), then Vulkan or OpenGL. `crash.log` notes each renderer that did not open. The
  `WGPU_BACKEND` variable sets one renderer.
- The Windows, Mac and Linux files contain `THIRD-PARTY-LICENSES.txt`, the licences of the Rust
  packages in the program. The browser version has them at the end of the `.html` file.
- The build uses GitHub actions pinned by commit, and checks the checksum of trunk.

## 2.0.0-alpha.3 (2026-11-14)

- Each camera keeps its own mode, controls, orientation and region. A different camera shows its
  full image.
- Mac: **Exposure**, **White balance** and **Focus** can be automatic or locked, where the camera
  allows it.
- Mac: the camera gives the frame rate of the mode that you pick, in full-range video.
- CONTRIBUTING.md, STATUS.md and issue forms. `laser report` writes a bug report.

## 2.0.0-alpha.2 (2026-11-14)

- A Mac app (macOS 11 and newer, Apple silicon and Intel) with live cameras.
- A Linux app (x86-64, glibc 2.35 and newer) with live cameras, the camera modes, the controls and
  **Lock for measuring**.
- GitHub Actions builds every release file after the tests pass, with `SHA256SUMS.txt`.
- The Synthetic camera acts like a block moved over a surface that is about 8 µm out of flat.
  **Wait** says Ready in about 5 minutes.

## 2.0.0-alpha.1 (2026-11-14)

A new program, written in Rust. The four steps are **Set up**, **Wait**, **Measure** and
**Results**. The [release notes](https://github.com/bhowiebkr/laser-level-webcam/releases/tag/2.0.0-alpha.1)
list what is new since 1.x.

## 1.0.1 (2023-06-30)

The last Python version. Its code is on the
[`python` branch](https://github.com/bhowiebkr/laser-level-webcam/tree/python).

## 1.0.0 (2023-06-30)

The first release, on PyPI.

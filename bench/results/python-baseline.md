# Python tool, baseline on synthetic frames, 2026-11-13

## Set-up

- Python tool v1.0.0 from `main` (f4fda43), no code changes. Python 3.11.0, PySide6 6.12.0, NumPy 2.4.6, SciPy 1.17.1, Matplotlib 3.11.2.
- Windows 11 Pro, Intel Core (family 6 model 183), 32 logical CPUs. Qt platform `offscreen`.
- `bench/python/run.py synthetic`, 355 s. Raw data: `python-synthetic-2026-11-13.json`.
- Frames: spec v1 from `bench/python/synthetic.py`. 1920x1080, vertical Gaussian line, sigma 80 px, background 16, amplitude 160, read noise 2, shot gain 0.5, seed 1. Each frame goes into the tool as a new NV12 `QVideoFrame`. SHA-256 of frame 0: `80d0e329…2fd330` (all four in the JSON).
- The harness calls the tool's own `FrameWorker.setVideoFrame`, `SampleWorker`, `Core.received_sample` and `Graph`. The time of each step comes from a copy of `setVideoFrame` with timers. Its sum (24.9 ms) agrees with the real method (25.1 ms).
- One thread, no camera, no painting. The GUI tool was open on the same PC during the run and used about one core of 32, so the times can be a little high.
- Scale for µm: sensor width 5.9 mm / 1920 px = 3.07 µm/px. Default smoothing k = 50.

## Results

### Time per frame

| Step | Mean ms | p99 ms |
|---|---|---|
| `toImage` and grey conversion | 15.16 | 17.48 |
| Rotate the full frame to a pixmap, for the feed | 4.39 | 5.98 |
| `np.mean(axis=0)` | 1.66 | 2.01 |
| Scope image loop | 1.39 | 1.75 |
| Scope pixmap flip | 1.22 | 1.81 |
| Gaussian fit (`curve_fit`) | 0.88 | 1.30 |
| Smoothing | 0.11 | 0.21 |
| Rescale to `uint8` | 0.06 | 0.12 |
| Other | 0.01 | 0.03 |
| **`setVideoFrame`, real method** | **25.14** | **27.57** |

Highest frame rate on one thread: **40 fps**. A 30 fps camera leaves 7.5 ms of margin per frame. Image work (conversion, feed pixmap, scope) is 22.2 ms, 88 % of the time. The fit is 3.5 %.

### Fit time with and without a line

| Case | Fit ms (p99) | Found |
|---|---|---|
| Line at the middle (960 px) | 0.85 (1.10) | 960.0 px |
| Line near the edge (1728 px) | 0.51 (0.85) | 959.5 px, which is wrong by 768 px |
| No line | 3.30 (7.91) | 992 px (sd 112), with no warning |

The live profile gave the fit 14.5 % of one core. If the camera gave 30 fps, that is about 4.8 ms per frame, which is near the "no line" case. Probably the laser was not on the sensor during the profile, or the real beam is far from a Gaussian.

### Sub-pixel accuracy

The line moves from 960.0 to 961.0 px in 0.1 px steps.

- The float centre from the fit is within ±0.02 px of the true centre.
- The value that leaves the frame worker goes through `Signal(int)` (bug 1, #43). It is the float centre cut to an integer, so the error of each value is minus the fractional part, between 0 and -1 px. Over the sweep it is -0.5 px on average. Near a whole pixel, the noise makes it jump between 0 and -1 px (mean -0.5 at 960.0, -0.6 at 961.0). That is up to -3.1 µm, and -1.5 µm on average, before any other error.

### Linearity across the sensor

20 positions from 5 % to 95 % of the width (96 to 1824 px).

| Sigma | Positions where the fit is correct | Worst error | Failures |
|---|---|---|---|
| 20 px | 824 to 1096 px (±136 px from the middle) | 864 px | 5 of 20 |
| 80 px | 642 to 1278 px (±318 px) | 864 px | 5 of 20 |
| 200 px | 278 to 1642 px (±682 px) | 864 px | 15 of 20 |

Outside the working range, the fit returns its start value (the middle of the profile) or fails. A narrow line, which is the best case for a precise measurement, has the smallest range: 14 % of the sensor. This is bug 18.

### Repeatability

30 samples of 10 subsamples, 30 % outliers, line at 960.37 px.

- The tool: sd 0.000 px, error -0.37 px (-1.14 µm) on each sample, from bug 1.
- The same outlier removal on the float centres: sd 0.006 px (0.02 µm), error -0.02 px.

The synthetic noise is pixel noise only. The tool averages 1080 rows, so this noise almost goes away. In real use, the beam moves (air, heat, vibration), and this is the largest error. Real frames from the live run must give this number.

### Scale

Zero with the line at 700 px, Take Sample with the line at 1200 px. True height: 1536.5 µm.

| Smoothing k | Reported | Error |
|---|---|---|
| 0 | 1537.2 µm | +0.05 % |
| 50 (default) | 1620.5 µm | +5.47 % |
| 200 | 1936.9 µm | +26.06 % |

With smoothing changed from 50 to 0 after Zero, and no motion, the tool reports +154.0 µm. This is bug 4. The README tip "crank the smoothing way up" makes the scale error larger.

### Memory

60 s, 2612 frames: 197 to 198 MB, slope of the second half -0.25 MB/min. No leak in this time. The 10-minute run is still to do.

### Graph

| Samples | 1 | 3 | 10 | 30 | 50 | 100 |
|---|---|---|---|---|---|---|
| Update ms | 120.6 | 123.1 | 127.7 | 100.7 | 95.7 | 91.5 |

Each new sample costs about 0.1 s of main-thread time for the Matplotlib redraw. Memory stays at 230 MB over clear-and-fill cycles.

## What this tells the Rust tool

The tool is a straightness and flatness gauge (see "What the tool is for" in `PLAN.md`). So the numbers that matter most are accuracy and trust, not speed.

1. **Accuracy is broken in three ways that the user cannot see:** the integer cut (bias up to -3.1 µm), the scale error from smoothing (+5.5 % at the default), and the fit range (wrong by hundreds of pixels outside the middle of the sensor). Each one gets a test in `laser-core`.
2. **The tool gives a number when it should give a warning.** With no line it reports 992 px. With the line near the edge it reports the middle. The Rust reading must carry flags (no beam, beam at the edge, clipped) and an uncertainty.
3. **The measurement itself is cheap.** The fit is 0.9 ms. Most of the time goes to image work for display: grey conversion and a full-frame rotation (19.6 ms of 25.1 ms). The Rust tool reads luma straight from the camera plane and lets the GPU rotate and scale the feed.
4. **Repeatability needs real frames.** Pixel noise is not the limit. Beam motion is. Synthetic frames spec v2 must add beam motion, tilt and clipping, and the live run must record real frames.

## Not known

- Why the grey conversion takes 15 ms here but about 2.2 ms per frame in the live profile (6.6 % core, if the camera gave 30 fps). The synthetic frames are NV12 in CPU memory; the camera can give another format. The live run records the format and the frame rate.
- The live frame rate, and how many camera frames the worker drops.
- Memory over 10 minutes.

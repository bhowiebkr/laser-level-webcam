# Python tool, live profile, 2026-11-13

## Set-up

- Python tool v1.0.0 from `main` (f4fda43), no code changes. Python 3.11.0, PySide6 6.12.0, NumPy 2.4.6, SciPy 1.17.1.
- Windows 11 Pro. Camera "USB Camera", no lens (the normal set-up for this tool). It gives 1920x1080 at up to 30 fps in NV12 or MJPEG. The tool does not set a format, so Qt selects it. We do not know which format Qt used.
- `py-spy record --rate 200 --duration 30 --threads --format raw`. Raw data: `python-live-2026-11-13.txt` (2724 samples, 0 errors).
- py-spy records only samples where a Python thread runs. Time in C code (NumPy, SciPy, Qt) shows on the Python line that called it. Native threads (camera decode, Qt render) are not in the samples.

## Result

The whole process used **110 % of one core** (Task Manager style, 10 s mean), 320 MB, 107 threads. py-spy sees **45 % of one core** in Python threads. The other **65 %** is in native threads that py-spy cannot see: the Qt Multimedia camera pipeline (decode and format conversion) and rendering.

`% core` = samples / 6000 (one core busy for 30 s at 200 Hz).

### Frame worker thread: 37.4 % core

| Step | `Workers.py` line | % core |
|---|---|---|
| `fit_gaussian` (SciPy `curve_fit`) | 190 | 14.5 |
| `QPixmap.fromImage(...).transformed(rotate(-90))` of the full frame, for the feed | 147 | 8.0 |
| `frame.toImage().convertToFormat(Grayscale8)` | 140 | 6.6 |
| `np.mean(axis=0)` | 142 | 2.8 |
| Scope image loop in Python | 168-169 | 1.8 |
| Scope pixmap flip | 184 | 1.6 |
| Smoothing, rescale, other | 151-203 | 0.9 |

### Main thread: 8.0 % core

| Step | % core |
|---|---|
| Qt event loop (`app.exec`, painting in C++) | 5.3 |
| `PixmapWidget.setPixmap` | 1.3 |
| `PixmapWidget.paintEvent` (scales the full frame to the widget) | 0.7 |
| `AnalyserWidget` paint and data | 0.5 |

## What this tells the Rust tool

1. The Gaussian fit is the largest Python cost (39 % of the worker). A fit on about 1800 points with a good start value must take much less time. A log-parabola or centroid finder costs almost nothing.
2. Image work for display costs more than the measurement: the grey conversion, the 90° rotation of the full frame, and two pixmaps take 16 % core. The Rust tool reads luma straight from the NV12 or YUYV plane, and the GPU rotates and scales the feed texture.
3. The largest cost is not in Python: 65 % core in the camera pipeline. The Rust tool must ask the camera for raw NV12 or YUYV, not MJPEG, and must not convert to RGB.
4. We cannot give a time per frame from this profile, because we did not count the frames. If the camera gave 30 fps and the worker took all frames, the worker used about 12.5 ms per frame. If the camera gave 15 fps (auto exposure in low light can do this), it used about 25 ms. The synthetic harness (Phase 0, step 2) gives the time per step and the frame count.

## Not known

- If the laser was on the sensor during the 30 s. With no line, `curve_fit` can take more or fewer iterations.
- The camera format and the frame rate that Qt selected.

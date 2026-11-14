# Benchmarks

The benchmarks measure the Python tool first, so that each change in the Rust tool has a number to compare with. Python and Rust use the same frames.

## Python harness

`bench/python/run.py` runs the tool's own code from `src/`. It does not change that code.

Set-up (once), from the repository root, with Python 3.11:

```bash
python -m venv .venv
.venv/Scripts/python -m pip install -r requirements.txt psutil
```

On Linux and macOS, use `.venv/bin/python`.

### Synthetic frames

```bash
.venv/Scripts/python bench/python/run.py synthetic
```

This takes about 5 minutes. `--quick` takes about 1 minute. `--memory-seconds 600` does the 10-minute memory run.

It measures:

| Section | What |
|---|---|
| `timing` | Time of each step of `FrameWorker.setVideoFrame`, and of the real method. Highest frame rate on one thread |
| `fit_cases` | Time of the fit with the line at the centre, near the edge, and with no line |
| `subpixel` | Error at 0.1 px steps: the float centre, and the value that goes through `Signal(int)` |
| `linearity` | Error at 20 positions across the sensor, for three line widths |
| `repeatability` | Spread of 30 samples (10 subsamples, 30 % outliers) of one scene |
| `scale` | Zero, then Take Sample 500 px away, at smoothing 0, 50 and 200. Also smoothing changed after Zero |
| `memory` | Process memory while frames run |
| `graph` | Time of one graph update, and memory over clear-and-fill cycles |

### Live camera

Close the GUI tool first. It holds the camera and uses CPU.

```bash
.venv/Scripts/python bench/python/run.py live --seconds 30 --save-frames 20
```

It counts frames from the camera and frames that the worker processes, and it measures CPU use. It records the camera format that Qt selects. `--save-frames N` saves N raw luma frames to `bench/data/live/` (git-ignored) as `.y8` files (8-bit rows, no header) with `frames.json`.

### Output

Each run writes `bench/results/python-<mode>-<date>.json`. The reports in `bench/results/*.md` explain the results.

## Synthetic frames

`bench/python/synthetic.py` makes each frame from a few numbers (centre, line width, background, amplitude, noise, seed, frame index). Its docstring gives the full specification, so that the Rust benchmarks can make the same bytes. The JSON output has the SHA-256 of the first frames to check this.

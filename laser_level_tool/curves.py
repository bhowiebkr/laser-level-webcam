from scipy.optimize import curve_fit
import numpy as np


def fit_gaussian(curve):
    try:

        def gaussian(x, mean):
            amplitude = np.max(curve)
            stddev = np.std(curve)

            # Check if stddev is zero
            if stddev == 0:
                return 0

            return amplitude * np.exp(-(((x - mean) / stddev) ** 2))

        x_data = np.arange(curve.size)
        y_data = np.asarray(curve)
        popt, _ = curve_fit(
            gaussian,
            x_data,
            y_data,
            p0=(np.mean(x_data),),
            absolute_sigma=True,
            maxfev=1000,
        )
        return popt[0]
    except:
        return None


def fit_gaussian_fast(curve):
    curve_max = np.max(curve)
    curve_std = np.nanstd(curve)
    if np.isnan(curve_std) or curve_max == 0 or curve_std == 0:
        return None

    def gaussian(x, mean):
        return curve_max * np.exp(-(((x - mean) / curve_std) ** 2))

    x_data = np.arange(curve.size)
    try:
        popt, _ = curve_fit(gaussian, x_data, curve, p0=(np.mean(x_data),), maxfev=800)
    except RuntimeError:
        return None
    else:
        return popt[0]

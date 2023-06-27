import numpy as np
from scipy.optimize import curve_fit


def fit_gaussian(curve):
    """
    Fits a Gaussian curve to the given data points.

    Args:
    curve: 1D array of float, representing the curve to be fitted.

    Returns:
    A float representing the mean of the fitted Gaussian curve.
    If the curve cannot be fitted, None is returned.
    """
    # Compute the maximum and standard deviation of the curve
    curve_max = np.max(curve)
    curve_std = np.nanstd(curve)

    # Check if the standard deviation is NaN or the curve max/std is zero
    if np.isnan(curve_std) or curve_max == 0 or curve_std == 0:
        return None

    # Define the Gaussian function
    def gaussian(x, mean):
        return curve_max * np.exp(-(((x - mean) / curve_std) ** 2))

    # Generate x data points and try to fit the curve using the defined
    # Gaussian function
    x_data = np.arange(curve.size)
    try:
        popt, _ = curve_fit(gaussian, x_data, curve, p0=(np.mean(x_data),), maxfev=800)
    except RuntimeError:
        # If the curve fitting fails, return None
        return None
    else:
        # Return the mean of the fitted Gaussian curve
        return popt[0]

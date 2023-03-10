import numpy as np
from scipy.stats import linregress

from PySide6.QtMultimedia import QMediaDevices

units_of_measurements = {"μm": 1000, "mm": 1, '0.0000"': 0.0393701, '0.00000"': 0.0393701}


def get_units(unit, value):
    if unit == '0.0000"':
        return f'{"{:.4f}".format(value * 0.0393701)}"'
    if unit == '0.00000"':
        return f'{"{:.5f}".format(value * 0.0393701)}"'

    if unit in units_of_measurements.keys():
        return f'{"{:.2f}".format(value * units_of_measurements[unit])}{unit}'


def adjust_image(image, brightness=0, contrast=1, gamma=1):
    """
    Adjusts the brightness, contrast, and gamma of a grayscale image using NumPy.
    """

    # Compute the scaling factor for brightness
    brightness_factor = 1.0 + (brightness * 100) / 255.0

    # Compute the scaling factor for contrast
    contrast_factor = (contrast - 1.0) / 255.0 * image.mean() + 1.0

    # Compute the gamma factor
    gamma_factor = 1.0 / gamma

    # Apply brightness, contrast, and gamma correction in one step
    adjusted_image = np.power(contrast_factor * image * brightness_factor / 255.0, gamma_factor) * 255.0

    # Clip the pixel values to the valid range of 0 to 255
    adjusted_image = np.clip(adjusted_image, 0, 255).astype(np.uint8)

    return adjusted_image


def get_webcam_max_res(index=0):
    available_cameras = QMediaDevices.videoInputs()
    if not available_cameras:
        return None

    camera_info = available_cameras[index]
    supported_resolutions = camera_info.photoResolutions()
    max_resolution = None
    for resolution in supported_resolutions:
        if max_resolution is None or (resolution.width() * resolution.height()) > (max_resolution.width() * max_resolution.height()):
            max_resolution = resolution

    return [max_resolution.width(), max_resolution.height()]


def scale_center_point(sensor_width, data_width, center, zero, units):
    val = float(sensor_width) / float(data_width) * (center - zero) * units_of_measurements[units]
    # print(f"sensor width: {sensor_width}\ndata width: {data_width}\ncenter: {center}\nzero: {zero}\nunits: {units}\nunit scale: {units_of_measurements[units]}\nval: {val}\n\n")
    return val


def scale_center_point_no_units(sensor_width, data_width, center, zero):
    if data_width == 0:
        return None
    return (sensor_width / data_width) * (center - zero)

    val = float(sensor_width) / float(data_width) * (center - zero)
    # print(f"sensor width: {sensor_width}\ndata width: {data_width}\ncenter: {center}\nzero: {zero}\n")
    return val


def samples_recalc(samples):
    # check if there are at least three samples
    if len(samples) >= 3:
        # calculate the slope, intercept, and other parameters of a linear regression line
        x = np.linspace(0, len(samples) - 1, len(samples))
        y = np.array([s.y for s in samples])
        slope, intercept, r_value, p_value, std_err = linregress(x, y)

        # calculate the minimum and maximum errors
        minYError, maxYError = np.inf, -np.inf
        for s in samples:
            s.linYError = np.abs(s.y - (slope * s.x + intercept))
            if s.linYError > maxYError:
                maxYError = s.linYError
            if s.linYError < minYError:
                minYError = s.linYError

        # calculate the shim and scrape properties
        for s in samples:
            # make highest point zero for shimming, we are going to shim up all the low points to this height
            s.shim = maxYError - s.linYError
            # make lowest point zero for scraping, we are going to scrape off all the high areas
            s.scrape = s.linYError - minYError

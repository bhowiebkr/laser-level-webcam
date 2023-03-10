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


def scale_center_point_no_units(sensor_width, data_width, center, zero):
    if data_width == 0:
        return None
    return (sensor_width / data_width) * (center - zero)


def samples_recalc(samples):
    if len(samples) >= 3:
        x = [s.x for s in samples]
        y = [s.y for s in samples]

        slope, intercept, r_value, p_value, std_err = linregress(x, y)

        minYError = float("inf")
        maxYError = float("-inf")
        for s in samples:
            s.linYError = s.y - (slope * s.x + intercept)
            if s.linYError > maxYError:
                maxYError = s.linYError
            if s.linYError < minYError:
                minYError = s.linYError

        for s in samples:
            # make highest point zero for shimming, we are going to shim up all the low points to this height
            s.shim = maxYError - s.linYError
            # make lowest point zero for scraping, we are going to scrape off all the high areas
            s.scrape = s.linYError - minYError

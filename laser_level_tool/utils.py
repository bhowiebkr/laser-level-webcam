from scipy.stats import linregress

units_of_measurements = {
    "μm": 1000,
    "mm": 1,
    '0.0000"': 0.0393701,
    '0.00000"': 0.0393701,
}


def get_units(unit, value):
    """
    Converts a value from a small unit of measurement to a more readable format and appends the appropriate scaler.

    Args:
    - unit (str): The unit of measurement of the value in small units.
    - value (float): The value to be converted.

    Returns:
    - str: The converted value with an appropriate scaler appended.

    Example:
    - get_units('0.0000"', 1.2345) -> '0.0488"'
    """
    # If the unit is in inches with four decimal places, convert to inches and format to four decimal places.
    if unit == '0.0000"':
        return f'{"{:.4f}".format(value * 0.0393701)}"'

    # If the unit is in inches with five decimal places, convert to inches and format to five decimal places.
    if unit == '0.00000"':
        return f'{"{:.5f}".format(value * 0.0393701)}"'

    # If the unit is a known unit of measurement, convert to that unit and format to two decimal places.
    if unit in units_of_measurements.keys():
        return f'{"{:.2f}".format(value * units_of_measurements[unit])}{unit}'


def scale_sample_real_world(sensor_width, data_width, sample, zero):
    """
    Converts a sample measurement into a real-world measurement in millimeters.

    Args:
    - sensor_width (float): The width of the sensor in millimeters.
    - data_width (float): The total width that the sample is found in.
    - sample (float): The measured difference from the zero point in the sample.
    - zero (float): The offset we start the sample from.

    Returns:
    - float: The converted sample measurement in millimeters.

    Example:
    - scale_sample_real_world(35.9, 1200, 500, 100) -> 10.508333333333333
    """
    if data_width == 0:
        return None
    # Convert the sample measurement into millimeters using the sensor width, data width, and zero offset.
    return (sensor_width / data_width) * (sample - zero)


def samples_recalc(samples):
    """
    Recalculates the linear regression and errors of the given list of samples.

    Args:
    - samples (list): A list of Sample objects with x and y attributes.

    Returns:
    - None

    Example:
    - sample1 = Sample(1, 2)
      sample2 = Sample(2, 4)
      sample3 = Sample(3, 6)
      samples_recalc([sample1, sample2, sample3])
    """
    # Ensure that there are at least 3 samples to calculate the linear regression and errors.
    if len(samples) >= 3:
        # Get the x and y values from the samples.
        x = [s.x for s in samples]
        y = [s.y for s in samples]

        # Calculate the linear regression using the x and y values.
        slope, intercept, r_value, p_value, std_err = linregress(x, y)

        # Calculate the minimum and maximum y errors for each sample.
        minYError = float("inf")
        maxYError = float("-inf")
        for s in samples:
            s.linYError = s.y - (slope * s.x + intercept)
            if s.linYError > maxYError:
                maxYError = s.linYError
            if s.linYError < minYError:
                minYError = s.linYError

        # Calculate the shim and scrape values for each sample.
        for s in samples:
            # Make highest point zero for shimming, we are going to shim up all the low points to this height.
            s.shim = maxYError - s.linYError
            # Make lowest point zero for scraping, we are going to scrape off all the high areas.
            s.scrape = s.linYError - minYError

from __future__ import annotations

units_of_measurements = {
    "μm": 1000,
    "mm": 1,
    '0.0000"': 0.0393701,
    '0.00000"': 0.0393701,
}


def get_units(unit: str, value: float) -> str:
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
    elif unit == '0.00000"':
        return f'{"{:.5f}".format(value * 0.0393701)}"'

    # If the unit is a known unit of measurement, convert to that unit and format to two decimal places.
    elif unit in units_of_measurements.keys():
        return f'{"{:.2f}".format(value * units_of_measurements[unit])}{unit}'
    else:
        return "ERROR"


def scale_sample_real_world(sensor_width: int, data_width: int, sample: float, zero: float) -> float:
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
        return 0
    # Convert the sample measurement into millimeters using the sensor width, data width, and zero offset.
    return (sensor_width / data_width) * (sample - zero)

tooltips = {}

tooltips[
    "zero_btn"
] = """This will set the zero position that we take measurements from.\n\nWarning:\n
Pressing this button will always clear all the samples."""
tooltips["subsamples"] = "When taking a sample, this is the number of subsamples that we average together."
tooltips["outliers"] = "This is the percentage of outliers we remove from the subsamples."
tooltips[
    "sensor_width"
] = """The physical sensor width (the longer length).\n\nIf this was a HD 1920x1080 sensor,
this would be the 1920px side.\nMeasure by taking a top-down photo and calculating in software
the size of the sensor from a known measured length with calipers. \nhat way you don't damage the sensor."""

tooltips[
    "units"
] = """These are the physical units that the measurements will be in.

Note:
The sensor width will always be in millimeters so you`ll have to convert from your units of choice.
It`s very likely the sensor is in metric units anyway."""

tooltips[
    "samples"
] = """Takes a new sample and appends it to the list of existing samples.

The sample will be taken using the current subsamples and outliers.
While taking a sample it`s best to not have anything moving as that will add error to the result.

If you are working in an area that has high vibrations that are noticeable on the sensor feed,
increase the number of samples to ~100 and the outliers to ~50%. In more controlled environments
outliers of 20% is good. High subsamples that are over 100-200 haven`t shown to be a benefit from my testing."""

tooltips[
    "replace"
] = """This replaces the selected sample in the table below.

Please refer to the tooltip on the Take Sample button for more information on usage. """

tooltips[
    "smoothing"
] = """Smoothing is used to remove the high frequency noise in the luminosity view above.

Smoothing can also be helpful if your laser white center is being clipped and doesn`t
have a proper peak/max point. By smoothing a large amount you will effectively bring
back the peak giving the mid point detector a better chance of finding the center.

Warning:
If you are smoothing by a high value and the luminosity view is showing that it's
coming off the view, it might skew the resulting value. It is generally best to use
a small amount of smoothing and only use large amounts in special cases like this. """

tooltips[
    "analyser"
] = """This view shows the brightness in a row of pixels, the current center of a sample (green) and the
zero point of its set (red).

To modify the smoothness of the values, see the slider below."""

tooltips[
    "feed"
] = """The sensor feed view shows the live view from the webcam sensor for the camera selected

in the combo box below. The image is in gray scale and is the native resolution of the webcam sensor.
For example, If the sensor is FHD the image and corresponding data in the analyser will be 1920 pixels wide.

Modifying the image is best done with ffmpeg using the button below."""


tooltips[
    "cam_device"
] = """This button loads up all the available webcam device settings for the camera.

Notable things to adjust:
- disable auto exposure and auto white balance.  When we are taking samples we don`t
  want any part of the image changing.
- If you have some lights on in the room, enabling powerline frequency filters can help
  with buzzing LED or fluorescent lights.
- Brightness, contrast, gamma are useful to help get the luminosity in a good range.

These settings are not likely to be saved so it`s always best
to check these settings out when using the tool."""


tooltips[
    "cameras"
] = """This combo box lists all the available web cameras on the machine. If you are running on a laptop,
the internal webcam will show up first followed by the USB camera you are wanting to use.

Different cameras have different resolutions. The calculations seen in the analyser view will run
on pixel resolution width of the camera`s sensor."""


tooltips[
    "table"
] = """This table shows all the samples and derived information on those samples in the units specified above.
When replacing samples, select a sample row here.

The measured value is the raw value that has been converted in the physical distance specified above.

The flattened values have had the average slope removed from the measured values. This is because we
want to be measuring the flatness of a surface and not its slope. This value is calculated by
subtracting the linear regression error value from the measured value.

Scrape values are useful if you want to know much more you would need to scrape each sample to
get it to the lowest measured sample.

Shim values is how much you would have to shim each sample up to the highest measured sample"""


tooltips["raw"] = """This radio button will set the plot to display the raw measured values."""


tooltips["flat"] = """This radio button will set the plot to display the derived flattened values."""

tooltips[
    "plot"
] = """This plot will graph the samples out as either the raw measured values or the
flattened values.

The line indicator is helpful for seeing how the surface is sloped.
Selected samples in the table above will show a dark red vertical line to
indicate what sample is selected."""

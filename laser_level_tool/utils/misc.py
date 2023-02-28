import numpy as np
import imageio


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
    adjusted_image = (
        np.power(contrast_factor * image * brightness_factor / 255.0, gamma_factor)
        * 255.0
    )

    # Clip the pixel values to the valid range of 0 to 255
    adjusted_image = np.clip(adjusted_image, 0, 255).astype(np.uint8)

    return adjusted_image


def get_webcam_max_res():
    # Create a reader object for the webcam
    reader = imageio.get_reader("<video1>")

    # Get the max resolution
    max_width = 0
    max_height = 0
    for frame in reader:
        height, width, shape = frame.shape
        if width > max_width:
            max_width = width
        if height > max_height:
            max_height = height
        break

    # Close the reader object
    reader.close()

    return [max_width, max_height]

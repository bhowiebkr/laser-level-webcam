from __future__ import annotations

from typing import Any

import numpy as np
import qimage2ndarray
from curves import fit_gaussian
from DataClasses import FrameData
from PySide6.QtCore import QObject
from PySide6.QtCore import Signal
from PySide6.QtCore import Slot
from PySide6.QtGui import QImage
from PySide6.QtGui import QPixmap
from PySide6.QtGui import QTransform
from PySide6.QtMultimedia import QVideoFrame
from utils import get_units


class SampleWorker(QObject):  # type: ignore
    """
    A worker class to process a stream of samples and emit the calculated mean.

    Attributes:
        OnSampleReady (Signal): Signal emitted when a sample is processed and a new mean is calculated.
        OnSubsampleRecieved (Signal): Signal emitted when a new subsample is received and processed.

    Methods:
        sample_in: Process a new subsample.
        start: Start the worker with a given number of total samples and outlier percentage to remove.
    """

    OnSampleReady = Signal(float)
    OnSubsampleRecieved = Signal(int)

    def __init__(self) -> None:
        super().__init__(None)
        self.ready = True
        self.sample_array = np.empty((0,))
        self.total_samples = 0
        self.running_total = 0
        self.outlier_percent = 0.0
        self.started = False

    def sample_in(self, sample: float) -> None:
        """
        Process a new subsample by appending it to the array and emitting OnSubsampleRecieved.

        When the total number of subsamples is reached, the worker calculates the mean, removes the outlier
        percentage, and emits OnSampleReady with the new mean.

        Args:
            sample (float): A new subsample to process.
        """
        if not self.started:
            return

        # Append new value to array
        self.sample_array = np.append(self.sample_array, sample)

        # self.acclimated_samples += sample
        self.running_total += 1

        self.OnSubsampleRecieved.emit(self.running_total)

        if self.running_total == self.total_samples:
            # Calculate the number of outliers to remove
            n_outliers = int(len(self.sample_array) * self.outlier_percent / 2.0)

            # Sort and remove the outliers
            first, last = n_outliers, -n_outliers if n_outliers > 0 else None
            self.sample_array = self.sample_array[self.sample_array.argsort()][first:last]

            # Calculate new mean as float
            mean = np.mean(self.sample_array).astype(float)

            self.OnSampleReady.emit(mean)

            # reset
            self.sample_array = np.empty((0,))
            self.running_total = 0
            self.total_samples = 0
            self.started = False

    def start(self, total_samples: int, outlier_percent: float) -> None:
        """
        Start the worker with a given number of total samples and outlier percentage to remove.

        Args:
            total_samples (int): The total number of subsamples to process before emitting the mean.
            outlier_percent (float): The percentage of outliers to remove from the subsamples (0-100).
        """
        self.total_samples = total_samples
        self.outlier_percent = outlier_percent / 100.0
        self.started = True


class FrameWorker(QObject):  # type: ignore
    """
    A worker class to process a QVideoFrame and emit the corresponding image data.

    Attributes:
        OnFrameChanged (Signal): Signal emitted when the processed image data is ready.

    Methods:
        setVideoFrame(frame: QVideoFrame) -> None:
            Process a new QVideoFrame and emit the corresponding image data.

    """

    OnFrameChanged = Signal(list)
    OnCentreChanged = Signal(int)
    OnPixmapChanged = Signal(QPixmap)
    OnAnalyserUpdate = Signal(FrameData)

    def __init__(self, parent_obj: Any):
        super().__init__(None)
        self.ready = True
        self.analyser_smoothing = 0
        self.centre = 0.0
        self.analyser_widget_height = 0
        self.parent_obj = parent_obj
        self.data_width = 0

    @Slot(QVideoFrame)  # type: ignore
    def setVideoFrame(self, frame: QVideoFrame) -> None:
        """
        Process a new QVideoFrame and emit the corresponding image data.

        Args:
            frame (QVideoFrame): A QVideoFrame object to be processed.

        Returns:
            None

        """
        self.ready = False

        # Get the frame as a gray scale image
        image = frame.toImage().convertToFormat(QImage.Format_Grayscale8)
        try:
            histo = np.mean(qimage2ndarray.raw_view(image), axis=0)
        except ValueError as e:
            print("Invalid QImage:", e)
            return

        pixmap = QPixmap.fromImage(image).transformed(QTransform().rotate(-90))
        self.OnPixmapChanged.emit(pixmap)

        # Smoothing
        kernel = np.ones(2 * self.analyser_smoothing + 1) / (2 * self.analyser_smoothing + 1)
        histo = np.convolve(histo, kernel, mode="valid")

        # Find the min and max values
        min_value, max_value = histo.min(), histo.max()

        # Rescale the intensity values to have a range between 0 and 255
        histo = ((histo - min_value) * (255.0 / (max_value - min_value))).clip(0, 255).astype(np.uint8)

        # Generate the image
        # Define the scope image data as the width (long side) of the image x 256 for pixels
        scopeData = np.zeros((histo.shape[0], 256), dtype=np.uint8)

        # Replace NaN values with 0
        self.histo = np.nan_to_num(histo)

        # Set scope data
        for i, intensity in enumerate(histo):
            scopeData[i, : int(intensity)] = 128

        # Create QImage directly from the scope data
        qimage = QImage(
            scopeData.data,
            scopeData.shape[1],
            scopeData.shape[0],
            scopeData.strides[0],
            QImage.Format_Grayscale8,
        )

        # Create QPixmap from QImage
        a_pix = QPixmap.fromImage(qimage)

        # Create a vertical flip transform and apply it to the QPixmap
        a_pix = a_pix.transformed(QTransform().scale(1, -1))

        width = self.histo.shape[0]
        self.data_width = width

        a_sample = 0
        self.centre = fit_gaussian(self.histo)  # Specify the y position of the line
        self.OnCentreChanged.emit(self.centre)
        if self.centre:
            # self.sample_worker.sample_in(self.centre)  # send the sample to the sample worker right away.
            a_sample = int(self.analyser_widget_height - self.centre * self.analyser_widget_height / width)

        a_zero, a_text = 0, ""
        if self.parent_obj.zero and self.centre:  # If we have zero, we can set it and the text
            a_zero = int(self.analyser_widget_height - self.parent_obj.zero * self.analyser_widget_height / width)
            centre_real = (self.parent_obj.sensor_width / width) * (self.centre - self.parent_obj.zero)
            a_text = get_units(self.parent_obj.units, centre_real)

        frame_data = FrameData(a_pix, a_sample, a_zero, a_text)
        self.OnAnalyserUpdate.emit(frame_data)

        # self.OnFrameChanged.emit([pixmap, histo, a_pix])
        self.ready = True


class FrameSender(QObject):  # type: ignore
    """
    A class to send QVideoFrames.

    Attributes:
        OnFrameChanged (Signal): Signal emitted when a new QVideoFrame is ready to be processed.
    """

    OnFrameChanged = Signal(QVideoFrame)

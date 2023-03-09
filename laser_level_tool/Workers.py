from PySide6.QtCore import Signal, QObject, Slot
from PySide6.QtGui import QImage, QPixmap, QTransform
from PySide6.QtMultimedia import QVideoFrame

import numpy as np
import qimage2ndarray


class SampleWorker(QObject):
    """
    A worker that does multisampling
    """

    OnSampleReady = Signal(float)
    OnSubsampleRecieved = Signal(int)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.ready = True
        self.sample_array = np.empty((0,))
        self.total_samples = 0
        self.running_total = 0
        self.outlier_percent = 0
        self.started = False

    def sample_in(self, sample):
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
            self.sample_array = self.sample_array[self.sample_array.argsort()][n_outliers:-n_outliers]

            # Calculate new mean as float
            mean = np.mean(self.sample_array).astype(float)

            self.OnSampleReady.emit(mean)

            # reset
            self.sample_array = np.empty((0,))
            self.running_total = 0
            self.total_samples = 0
            self.started = False

    def start(self, total_samples, outlier_percent):
        self.total_samples = total_samples
        self.outlier_percent = outlier_percent / 100
        self.started = True


class FrameWorker(QObject):
    """
    FrameWorker gets frames from the webcam as a pixmap and numpy array [pixmap, np.array]
    """

    OnFrameChanged = Signal(list)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.ready = True
        self.analyser_smoothing = 0

    @Slot(QVideoFrame)
    def setVideoFrame(self, frame: QVideoFrame):
        """
        Recieves the fram from the FrameSender
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
        qimage = QImage(scopeData.data, scopeData.shape[1], scopeData.shape[0], scopeData.strides[0], QImage.Format_Grayscale8)

        # Create QPixmap from QImage
        a_pix = QPixmap.fromImage(qimage)

        # Create a vertical flip transform and apply it to the QPixmap
        a_pix = a_pix.transformed(QTransform().scale(1, -1))

        self.OnFrameChanged.emit([pixmap, histo, a_pix])
        self.ready = True

    def set_smoothness(self, smoothness):
        self.analyser_smoothing = smoothness


class FrameSender(QObject):
    """
    A frame sender that sends the frame to the FrameWorker
    """

    OnFrameChanged = Signal(QVideoFrame)

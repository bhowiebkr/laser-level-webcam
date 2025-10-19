from __future__ import annotations

import numpy as np
from PySide6.QtCore import QObject
from PySide6.QtCore import QThread
from PySide6.QtCore import Signal
from PySide6.QtCore import Slot
from PySide6.QtGui import QPixmap
from PySide6.QtMultimedia import QCamera
from PySide6.QtMultimedia import QMediaCaptureSession
from PySide6.QtMultimedia import QMediaDevices
from PySide6.QtMultimedia import QVideoFrame
from PySide6.QtMultimedia import QVideoSink
from scipy.stats import linregress

from src.DataClasses import Sample
from src.Workers import FrameSender
from src.Workers import FrameWorker
from src.Workers import SampleWorker


def samples_recalc(samples: list[Sample]) -> None:
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


class Core(QObject):  # type: ignore
    OnSensorFeedUpdate = Signal(QPixmap)
    OnAnalyserUpdate = Signal(list)
    OnSubsampleProgressUpdate = Signal(list)
    OnSampleComplete = Signal()
    OnUnitsChanged = Signal(str)

    def __init__(self) -> None:
        super().__init__()

        self.pixmap = None  # pixmap used for the camera feed
        self.histo = None  # histogram values used in analyser
        self.camera = QCamera()  # camera being used
        self.centre = 0.0  # The found centre of the histogram
        self.zero = 0.0  # The zero point
        self.analyser_widget_height = 0  # The height of the widget so we can calculate the offset
        self.subsamples = 0  # total number of subsamples
        self.outliers = 0  # percentage value of how many outliers to remove from a sample
        self.units = ""  # string representing the units
        self.sensor_width = 0  # width of the sensor in millimeters (mm)
        self.setting_zero_sample = False  # boolean if we are setting zero or a sample
        self.replacing_sample = False  # If we are replacing a sample
        self.replacing_sample_index = 0  # the index of the sample we are replacing
        self.line_data = np.empty(0)  # numpy array of the fitted line through the samples
        self.samples: list[Sample] = []

        # Frame worker
        self.workerThread = QThread()
        self.captureSession = QMediaCaptureSession()
        self.frameSender = FrameSender()
        self.frameWorker = FrameWorker(parent_obj=self)
        self.frameWorker.moveToThread(self.workerThread)
        self.workerThread.start()

        # Sample worker
        self.sample_worker = SampleWorker()
        self.sample_worker.OnSampleReady.connect(self.received_sample)
        self.sample_worker.OnSubsampleRecieved.connect(self.subsample_progress_update)
        self.sampleWorkerThread = QThread()
        self.sample_worker.moveToThread(self.sampleWorkerThread)
        self.sampleWorkerThread.start()

        self.captureSession.setVideoSink(QVideoSink(self))
        self.captureSession.videoSink().videoFrameChanged.connect(self.onFramePassedFromCamera)
        self.frameSender.OnFrameChanged.connect(self.frameWorker.setVideoFrame)

    def delete_sample(self, index: int) -> None:
        debug = False
        if debug:
            print(f"num samples = {len(self.samples)}")
            print(f"Before: {self.samples=}")

        del self.samples[index]

        # Fix the indexes
        for index, sample in enumerate(self.samples):
            sample.x = index

        samples_recalc(self.samples)

        if debug:
            print(f"After: {self.samples=}")

    def subsample_progress_update(self, subsample: Sample) -> None:
        self.OnSubsampleProgressUpdate.emit([subsample, self.subsamples])  # current sample and total

    def received_sample(self, val: float) -> None:
        if self.setting_zero_sample:
            self.zero = val
        else:
            size_in_mm = (self.sensor_width / self.frameWorker.data_width) * (val - self.zero)

            if self.replacing_sample:
                x_orig = self.samples[self.replacing_sample_index].x
                self.samples[self.replacing_sample_index] = Sample(x=x_orig, y=size_in_mm)
                self.replacing_sample = False

            else:  # Append to samples
                self.samples.append(Sample(x=len(self.samples), y=size_in_mm))

            samples_recalc(self.samples)

        self.OnSampleComplete.emit()

    def set_units(self, units: str) -> None:
        self.units = units

        self.OnUnitsChanged.emit(self.units)

    def start_sample(self, zero: bool, replacing_sample: bool, replacing_sample_index: int) -> None:
        self.replacing_sample = replacing_sample
        self.replacing_sample_index = replacing_sample_index

        if zero:  # if we are zero, we reset everything
            self.line_data = np.empty(0)
            self.zero = 0.0

        self.setting_zero_sample = zero
        self.sample_worker.start(self.subsamples, self.outliers)

    @Slot(QVideoFrame)  # type: ignore
    def onFramePassedFromCamera(self, frame: QVideoFrame):
        if self.frameWorker.ready:
            self.frameSender.OnFrameChanged.emit(frame)

    def get_cameras(self) -> list[str]:
        cams = []
        for cam in QMediaDevices.videoInputs():
            cams.append(cam.description())

        return cams

    def set_camera(self, index: int) -> None:
        if self.camera:
            self.camera.stop()

        available_cameras = QMediaDevices.videoInputs()
        if not available_cameras:
            return

        camera_info = available_cameras[index]
        self.camera = QCamera(cameraDevice=camera_info, parent=self)

        self.captureSession.setCamera(self.camera)
        self.camera.start()

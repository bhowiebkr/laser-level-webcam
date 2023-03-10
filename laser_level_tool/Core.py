from PySide6.QtCore import QObject, QThread, Slot, Signal
from PySide6.QtMultimedia import QMediaCaptureSession, QVideoSink, QVideoFrame, QMediaDevices, QCamera
from PySide6.QtGui import QPixmap

import numpy as np
from Workers import FrameSender, FrameWorker, SampleWorker
from curves import fit_gaussian
from utils import get_units, samples_recalc


class Sample:
    def __init__(self, x, y):
        self.x = x
        self.y = y
        self.linYError = 0
        self.shim = 0
        self.scrape = 0


class Core(QObject):
    OnSensorFeedUpdate = Signal(QPixmap)
    OnAnalyserUpdate = Signal(list)
    OnSubsampleProgressUpdate = Signal(list)
    OnSampleComplete = Signal()
    OnUnitsChanged = Signal(str)

    def __init__(self):
        super().__init__()

        self.pixmap = None  # pixmap used for the camera feed
        self.histo = None  # histogram values used in analyser
        self.camera = None  # camera being used
        self.centre = None  # The found centre of the histogram
        self.zero = None  # The zero point
        self.analyser_widget_height = 0  # The height of the widget so we can calculate the offset
        self.subsamples = None  # total number of subsamples
        self.outliers = None  # percentage value of how many outliers to remove from a sample
        self.units = None  # string representing the units
        self.sensor_width = None  # width of the sensor in millimeters (mm)
        self.setting_zero_sample = False  # boolean if we are setting zero or a sample
        self.replacing_sample = False  # If we are replacing a sample
        self.replacing_sample_index = None  # the index of the sample we are replacing
        self.sample_data = np.empty(0)  # numpy array of raw samples
        self.line_data = np.empty(0)  # numpy array of the fitted line through the samples
        self.samples = []

        # Frame worker
        self.workerThread = QThread()
        self.captureSession = QMediaCaptureSession()
        self.frameSender = FrameSender()
        self.frameWorker = FrameWorker()
        self.frameWorker.moveToThread(self.workerThread)
        self.workerThread.start()

        # Sample worker
        self.sample_worker = SampleWorker()
        self.sample_worker.OnSampleReady.connect(self.received_sample)
        self.sample_worker.OnSubsampleRecieved.connect(self.subsample_progress_update)
        self.sampleWorkerThread = QThread()
        self.sample_worker.moveToThread(self.sampleWorkerThread)
        self.sampleWorkerThread.start()
        self.sample_worker.sample_in

        self.captureSession.setVideoSink(QVideoSink(self))
        self.captureSession.videoSink().videoFrameChanged.connect(self.onFramePassedFromCamera)
        self.frameSender.OnFrameChanged.connect(self.frameWorker.setVideoFrame)
        self.frameWorker.OnFrameChanged.connect(self.set_frame)

    def subsample_progress_update(self, subsample):
        self.OnSubsampleProgressUpdate.emit([subsample, self.subsamples])  # current sample and total

    def received_sample(self, val):
        if self.setting_zero_sample:
            self.zero = val
        else:
            size_in_mm = (self.sensor_width / self.histo.shape[0]) * (val - self.zero)

            if self.replacing_sample:
                x_orig = self.samples[self.replacing_sample_index].x
                self.samples[self.replacing_sample_index] = Sample(x=x_orig, y=size_in_mm)
                self.replacing_sample = False

            else:  # Append to samples
                self.samples.append(Sample(x=len(self.samples), y=size_in_mm))

            samples_recalc(self.samples)

        self.OnSampleComplete.emit()

    def set_units(self, units):
        self.units = units

        self.OnUnitsChanged.emit(self.units)

    def start_sample(self, zero, replacing_sample, replacing_sample_index):
        self.replacing_sample = replacing_sample
        self.replacing_sample_index = replacing_sample_index

        if zero:  # if we are zero, we reset everything
            self.sample_data = np.empty(0)
            self.line_data = np.empty(0)
            self.zero = None

        self.setting_zero_sample = zero
        self.sample_worker.start(self.subsamples, self.outliers)

    @Slot(QVideoFrame)
    def onFramePassedFromCamera(self, frame: QVideoFrame):
        if self.frameWorker.ready:
            self.frameSender.OnFrameChanged.emit(frame)

    def set_frame(self, data):
        """
        This is where most of the data processing happens
        """
        self.pixmap, self.histo, a_pix = data
        self.OnSensorFeedUpdate.emit(self.pixmap)

        width = self.histo.shape[0]

        a_sample = None
        self.centre = fit_gaussian(self.histo)  # Specify the y position of the line
        if self.centre:
            self.sample_worker.sample_in(self.centre)  # send the sample to the sample worker right away.
            a_sample = int(self.analyser_widget_height - self.centre * self.analyser_widget_height / width)

        a_zero, a_text = None, None
        if self.zero and self.centre:  # If we have zero, we can set it and the text
            a_zero = int(self.analyser_widget_height - self.zero * self.analyser_widget_height / width)
            centre_real = (self.sensor_width / width) * (self.centre - self.zero)
            a_text = get_units(self.units, centre_real)

        self.OnAnalyserUpdate.emit([a_pix, a_sample, a_zero, a_text])

    def get_cameras(self):
        cams = []
        for cam in QMediaDevices.videoInputs():
            cams.append(cam.description())
        return cams

    def set_camera(self, index):
        if self.camera:
            self.camera.stop()

        available_cameras = QMediaDevices.videoInputs()
        if not available_cameras:
            return

        camera_info = available_cameras[index]
        self.camera = QCamera(cameraDevice=camera_info, parent=self)

        self.captureSession.setCamera(self.camera)
        self.camera.start()

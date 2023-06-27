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
from utils import samples_recalc
from Workers import FrameSender
from Workers import FrameWorker
from Workers import SampleWorker


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
        self.frameWorker = FrameWorker(parent=self)
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
            size_in_mm = (self.sensor_width / self.frameWorker.data_width) * (val - self.zero)

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

        # This method shouldn't exist anymore

        self.pixmap, self.histo, a_pix = data
        self.OnSensorFeedUpdate.emit(self.pixmap)

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

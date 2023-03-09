from PySide6.QtCore import QObject, QThread, Slot, Signal
from PySide6.QtMultimedia import QMediaCaptureSession, QVideoSink, QVideoFrame, QMediaDevices, QCamera
from PySide6.QtGui import QImage, QPixmap, QTransform

import numpy as np
from Workers import FrameSender, FrameWorker, SampleWorker
from utils.curves import fit_gaussian
from utils.misc import scale_center_point_no_units, get_units


class Core(QObject):
    OnSensorFeedUpdate = Signal(QPixmap)
    OnAnalyserUpdate = Signal(list)
    OnSubsampleProgressUpdate = Signal(list)
    OnSampleComplete = Signal()

    def __init__(self):
        super().__init__()

        self.pixmap = None  # pixmap used for the camera feed
        self.histo = None  # histogram values used in analyser
        self.camera = None  # camera being used
        self.analyser_smoothing = 0  # Smoothing
        self.centre = None  # The found centre of the histogram
        self.zero = None  # The zero point
        self.analyser_widget_height = 0  # The height of the widget so we can calculate the offset
        self.subsamples = None  # total number of subsamples
        self.outliers = None  # percentage value of how many outliers to remove from a sample
        self.units = None  # string representing the units
        self.sensor_width = None  # width of the sensor in millimeters (mm)
        self.setting_zero_sample = False  # boolean if we are setting zero or a sample

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

    def received_sample(self, sample):
        self.OnSampleComplete.emit()

        if self.setting_zero_sample:
            self.zero = sample
        else:
            # do something with the sample. store in list
            pass

    def set_subsamples(self, samples):
        self.subsamples = samples

    def set_outliers(self, outliers):
        self.outliers = outliers

    def set_units(self, units):
        self.units = units

    def set_sensor_width(self, width):
        self.sensor_width = width

    def start_sample(self, zero=False):
        if zero:
            self.zero = None
        self.setting_zero_sample = zero
        self.sample_worker.start(self.subsamples, self.outliers)

    def set_analyser_widget_height(self, height):
        self.analyser_widget_height = height

    def set_analyser_smoothing(self, smoothing):
        self.analyser_smoothing = smoothing

    @Slot(QVideoFrame)
    def onFramePassedFromCamera(self, frame: QVideoFrame):
        if self.frameWorker.ready:
            self.frameSender.OnFrameChanged.emit(frame)

    def set_frame(self, data):
        """
        This is where most of the data processing happens
        """
        self.pixmap = data[0]
        self.histo = data[1]

        # Sensor Feed
        self.OnSensorFeedUpdate.emit(self.pixmap)

        # Smoothing
        # compute the moving average with nearest neighbour
        kernel = np.ones(2 * self.analyser_smoothing + 1) / (2 * self.analyser_smoothing + 1)
        self.histo = np.convolve(self.histo, kernel, mode="valid")

        # Find the min and max values
        min_value = np.min(self.histo)
        max_value = np.max(self.histo)

        # Ensure max_value and min_value are not equal to avoid division by zero
        if max_value == min_value:
            max_value += 1
        # Rescale the intensity values to have a range between 0 and 255
        self.histo = (self.histo - min_value) * (255 / (max_value - min_value))

        # Generate the image
        # Defind the scope image data as the width (long side) of the image x 256 for pixels
        scopeData = np.zeros((self.histo.shape[0], 256)).astype(np.uint8)

        # Loop over intensity values and set scope data
        for i, intensity in enumerate(self.histo):
            if np.isnan(intensity):
                intensity = 0
            try:
                scopeData[i, : int(intensity)] = 128
            except IndexError as e:
                print(e)

        qimage = QImage(
            scopeData,
            scopeData.shape[1],
            scopeData.shape[0],
            QImage.Format_Grayscale8,
        )

        a_pix = QPixmap.fromImage(qimage)

        # Create a vertical flip transform
        transform = QTransform()
        transform.scale(1, -1)
        a_pix = a_pix.transformed(transform)

        self.centre = fit_gaussian(self.histo)  # Specify the y position of the line
        self.sample_worker.sample_in(self.centre)
        width = self.histo.shape[0]
        a_sample = int(self.analyser_widget_height - self.centre * self.analyser_widget_height / width)

        a_zero = None
        a_text = None
        if self.zero:  # If we have zero, we can set it and the text
            a_zero = int(self.analyser_widget_height - self.zero * self.analyser_widget_height / width)
            center_point_real = scale_center_point_no_units(self.analyser_widget_height, width, self.centre, self.zero)
            a_text = get_units(self.units, center_point_real)

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

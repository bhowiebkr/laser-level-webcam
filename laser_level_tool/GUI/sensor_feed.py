import subprocess
import numpy as np

from PySide6.QtWidgets import QSizePolicy, QGroupBox, QVBoxLayout, QFormLayout, QSlider, QPushButton, QWidget, QComboBox
from PySide6.QtCore import Qt, QThread, Signal, QObject, Slot
from PySide6.QtGui import QPainter, QImage, QPixmap, QTransform
from PySide6.QtMultimedia import QMediaCaptureSession, QVideoSink, QVideoFrame, QCamera, QMediaDevices

import qimage2ndarray
from GUI.widgets import ResolutionInputWidget


class FrameWorker(QObject):
    pixmapChanged = Signal(QPixmap)
    intensityValuesChanged = Signal(np.ndarray)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.ready = True

    @Slot(QVideoFrame)
    def setVideoFrame(self, frame: QVideoFrame):
        self.ready = False

        # Get the frame as a gray scale image
        image = frame.toImage().convertToFormat(QImage.Format_Grayscale8)

        np_image = qimage2ndarray.raw_view(image)
        self.intensityValuesChanged.emit(np.mean(np_image, axis=0))

        # Get a pixmap rotated -90
        pixmap = QPixmap.fromImage(image).transformed(QTransform().rotate(-90))
        self.pixmapChanged.emit(pixmap)
        self.ready = True


class FrameSender(QObject):
    frameChanged = Signal(QVideoFrame)


# Define the left widget to display the grayscale webcam feed
class SensorFeedWidget(QWidget):
    height_changed = Signal(int)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.image = None
        self.pixmap = None
        self.camera = None
        self.captureSession = None

        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

        self.workerThread = QThread()
        self.captureSession = QMediaCaptureSession()
        self.frameSender = FrameSender()
        self.frameWorker = FrameWorker()

        self.frameWorker.moveToThread(self.workerThread)
        self.workerThread.start()

        self.captureSession.setVideoSink(QVideoSink(self))
        self.captureSession.videoSink().videoFrameChanged.connect(self.onFramePassedFromCamera)
        self.frameSender.frameChanged.connect(self.frameWorker.setVideoFrame)
        self.frameWorker.pixmapChanged.connect(self.setPixmap)

    @Slot(QVideoFrame)
    def onFramePassedFromCamera(self, frame: QVideoFrame):
        if self.frameWorker.ready:
            self.frameSender.frameChanged.emit(frame)

    def paintEvent(self, event):
        super().paintEvent(event)

        if not self.pixmap:
            return

        painter = QPainter(self)
        painter.drawPixmap(self.rect(), self.pixmap)

    def setPixmap(self, pixmap):
        self.pixmap = pixmap
        self.update()

    def resizeEvent(self, event):
        new_height = event.size().height()
        self.height_changed.emit(new_height)
        super().resizeEvent(event)

    def set_camera(self, index):
        if self.camera:
            self.camera.stop()

        available_cameras = QMediaDevices.videoInputs()
        camera_info = available_cameras[index]

        self.camera = QCamera(cameraDevice=camera_info, parent=self)

        self.captureSession.setCamera(self.camera)
        self.camera.start()
        self.camera.setExposureMode(QCamera.ExposureManual)
        print(self.camera.exposureMode())

    def closeEvent(self, event):
        super().closeEvent(event)


class SensorFeed(QGroupBox):
    def __init__(self, parent=None):
        super().__init__(parent)

        self.setTitle("Sensor Feed")

        # Layouts
        main_layout = QVBoxLayout()
        self.setLayout(main_layout)

        params = QFormLayout()

        # Widgets
        self.cameraPicker = QComboBox()
        self.get_cameras()

        self.widget = SensorFeedWidget()

        if self.cameraPicker.count():
            self.cameraPicker.currentIndexChanged.connect(self.widget.set_camera)
            self.widget.set_camera(0)

        self.exposure = QSlider(Qt.Horizontal)
        self.exposure.setMinimum(-200)
        self.exposure.setMaximum(200)
        self.exposure.setValue(0)

        self.contrast = QSlider(Qt.Horizontal)
        self.contrast.setMinimum(1)
        self.contrast.setMaximum(200)
        self.contrast.setValue(100)
        self.contrast.setTickInterval(1)

        self.gamma = QSlider(Qt.Horizontal)
        self.gamma.setMinimum(1)
        self.gamma.setMaximum(200)
        self.gamma.setValue(100)

        sensor_res = ResolutionInputWidget(None, 0, 0)
        sensor_res.lock()

        extra_btn = QPushButton("Camera Device Controls")
        extra_btn.setFixedHeight(40)

        # add widgets
        params.addRow("Camera", self.cameraPicker)
        # params.addRow("Exposure", self.exposure)
        # params.addRow("Contrast", self.contrast)
        # params.addRow("Gamma", self.gamma)
        # params.addRow("Feed Res", sensor_res)

        main_layout.addWidget(self.widget)
        main_layout.addLayout(params)
        main_layout.addWidget(extra_btn)

        # Logic
        extra_btn.clicked.connect(self.extra_controls)

    def get_cameras(self):
        for cam in QMediaDevices.videoInputs():
            name = cam.description()
            self.cameraPicker.addItem(name)

    def extra_controls(self):
        cmd = f'ffmpeg -f dshow -show_video_device_dialog true -i video="{self.cameraPicker.currentText()}"'
        subprocess.Popen(cmd, shell=True)

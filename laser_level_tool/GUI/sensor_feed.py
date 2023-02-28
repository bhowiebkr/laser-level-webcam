import imageio
import subprocess
import numpy as np

from PyQt5.QtWidgets import (
    QWidget,
    QSizePolicy,
    QGroupBox,
    QVBoxLayout,
    QFormLayout,
    QSlider,
    QPushButton,
)
from PyQt5.QtCore import Qt, QThread, pyqtSignal

from PyQt5.QtGui import QPainter, QImage, QPixmap, QTransform
from utils.misc import adjust_image
from GUI.widgets import ResolutionInputWidget


SIZE = [640, 480]
SIZE = [800, 600]


# Define the webcam thread to capture frames from the webcam and update the widgets
class WebcamThread(QThread):
    image_ready = pyqtSignal(np.ndarray)
    intensity_values_ready = pyqtSignal(np.ndarray)
    stop_signal = pyqtSignal()

    def __init__(self):
        super().__init__()
        self._is_running = True

        self.analyser = None
        self.sensor_feed = None

    def setAnalyser(self, analyser):
        self.analyser = analyser

    def setSensorFeed(self, sensor_feed):
        self.sensor_feed = sensor_feed

    def run(self):
        with imageio.get_reader("<video1>", size=(SIZE[0], SIZE[1])) as webcam:
            while self._is_running:
                # Read a frame from the webcam
                frame = webcam.get_next_data()

                # Brightness, contrast, gamma
                brightness = self.sensor_feed.brightness.value() / 100
                contrast = self.sensor_feed.contrast.value() / 100
                gamma = self.sensor_feed.gamma.value() / 100

                # Convert the RGB image to grayscale using the luminosity method
                image = np.dot(frame[..., :3], [0.2126, 0.7152, 0.0722]).astype(
                    np.uint8
                )
                image = adjust_image(image, brightness, contrast, gamma)

                self.image_ready.emit(image)

                intensity_values = np.mean(image, axis=0)

                self.intensity_values_ready.emit(intensity_values)

    def stop(self):
        self._is_running = False
        self.wait()


# Define the left widget to display the grayscale webcam feed
class SensorFeedWidget(QWidget):
    height_changed = pyqtSignal(int)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.image = None
        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

    def paintEvent(self, event):
        super().paintEvent(event)
        painter = QPainter(self)
        if self.image is not None:
            qimage = QImage(
                self.image.data,
                self.image.shape[1],
                self.image.shape[0],
                QImage.Format_Grayscale8,
            )
            pixmap = QPixmap.fromImage(qimage)
            pixmap = pixmap.transformed(QTransform().rotate(-90))
            painter.drawPixmap(self.rect(), pixmap)

    def setImage(self, image):
        self.image = image
        self.update()

    def resizeEvent(self, event):
        new_height = event.size().height()
        self.height_changed.emit(new_height)
        super().resizeEvent(event)


class SensorFeed(QGroupBox):
    def __init__(self, parent=None):
        super().__init__(parent)

        self.setTitle("Sensor Feed")

        # Layouts
        main_layout = QVBoxLayout()
        self.setLayout(main_layout)

        params = QFormLayout()

        # Widgets
        self.widget = SensorFeedWidget()

        self.brightness = QSlider(Qt.Horizontal)
        self.brightness.setMinimum(-200)
        self.brightness.setMaximum(200)
        self.brightness.setValue(0)

        self.contrast = QSlider(Qt.Horizontal)
        self.contrast.setMinimum(1)
        self.contrast.setMaximum(200)
        self.contrast.setValue(100)
        self.contrast.setTickInterval(1)
        self.gamma = QSlider(Qt.Horizontal)
        self.gamma.setMinimum(1)
        self.gamma.setMaximum(200)
        self.gamma.setValue(100)

        sensor_res = ResolutionInputWidget(None, SIZE[0], SIZE[1])
        sensor_res.lock()

        extra_btn = QPushButton("Camera Device Controls")
        extra_btn.setFixedHeight(40)

        # add widgets
        params.addRow("Brightness", self.brightness)
        params.addRow("Contrast", self.contrast)
        params.addRow("Gamma", self.gamma)
        params.addRow("Feed Res", sensor_res)

        main_layout.addWidget(self.widget)
        main_layout.addLayout(params)
        main_layout.addWidget(extra_btn)

        # Start the webcam thread
        self.webcam_thread = WebcamThread()

        # Logic
        extra_btn.clicked.connect(self.extra_controls)

    def extra_controls(self):
        cmd = 'ffmpeg -f dshow -show_video_device_dialog true -i video="USB Camera"'
        subprocess.Popen(cmd, shell=True)

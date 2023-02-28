import imageio
import threading
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
from PyQt5.QtCore import Qt, QThread

from PyQt5.QtGui import QPainter, QImage, QPixmap, QTransform
from utils.misc import adjust_image

SIZE = [640, 480]


# Define the webcam thread to capture frames from the webcam and update the widgets
class WebcamThread(threading.Thread):
    def __init__(self):
        super().__init__()
        self.stop_event = threading.Event()

        self.analyser = None
        self.sensor_feed = None

    def setAnalyser(self, analyser):
        self.analyser = analyser

    def setSensorFeed(self, sensor_feed):
        self.sensor_feed = sensor_feed

    def run(self):
        with imageio.get_reader("<video1>", size=(SIZE[0], SIZE[1])) as webcam:
            while not self.stop_event.is_set():
                # Read a frame from the webcam
                frame = webcam.get_next_data()

                # Brightness, contrast, gamma
                brightness = self.sensor_feed.brightness.value() / 100
                contrast = self.sensor_feed.contrast.value() / 100
                gamma = self.sensor_feed.gamma.value() / 100

                # Convert the RGB image to grayscale using the luminosity method
                gray = np.dot(frame[..., :3], [0.2126, 0.7152, 0.0722]).astype(np.uint8)
                gray = adjust_image(gray, brightness, contrast, gamma)

                intensity_values = np.mean(gray, axis=0)

                # Find the min and max values
                min_value = np.min(intensity_values)
                max_value = np.max(intensity_values)

                # Ensure max_value and min_value are not equal to avoid division by zero
                if max_value == min_value:
                    max_value += 1
                # Rescale the intensity values to have a range between 0 and 255
                intensity_values = (intensity_values - min_value) * (
                    255 / (max_value - min_value)
                )

                # Update the left and right widgets
                self.sensor_feed.widget.setImage(gray)
                self.analyser.widget.setLuminosityScope(intensity_values)
                self.analyser.widget.setFixedHeight(self.sensor_feed.widget.height())
                # Wait for a short time to avoid overloading the CPU
                self.stop_event.wait(0.001)

    def stop(self):
        self.stop_event.set()
        self.join()


# Define the left widget to display the grayscale webcam feed
class SensorFeedWidget(QWidget):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.image = None
        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

    def paintEvent(self, event):
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


class SensorFeed(QGroupBox):
    def __init__(self, parent):
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

        extra_btn = QPushButton("Camera Device Controls")
        extra_btn.setFixedHeight(40)

        # add widgets
        params.addRow("Brightness", self.brightness)
        params.addRow("Contrast", self.contrast)
        params.addRow("Gamma", self.gamma)

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

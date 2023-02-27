import imageio
import threading
import subprocess
import numpy as np

from PyQt5.QtWidgets import (
    QWidget,
    QSizePolicy,
    QGroupBox,
    QHBoxLayout,
    QVBoxLayout,
    QFormLayout,
    QSlider,
    QPushButton,
)
from PyQt5.QtCore import Qt

from PyQt5.QtGui import QPainter, QImage, QPixmap, QTransform

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
                # Convert the RGB image to grayscale using the luminosity method
                gray = np.dot(frame[..., :3], [0.2126, 0.7152, 0.0722]).astype(np.uint8)
                intensity_values = np.mean(gray, axis=0)

                # Smoothing
                try:
                    # compute the moving average with nearest neighbour
                    smoothingFactor = self.parent.smoothingSlider.value()
                    kernel = np.ones(2 * smoothingFactor + 1) / (
                        2 * smoothingFactor + 1
                    )
                    intensity_values = np.convolve(
                        intensity_values, kernel, mode="valid"
                    )
                except Exception:
                    pass

                # Find the min and max values
                min_value = np.min(intensity_values)
                max_value = np.max(intensity_values)

                # Scale the intensity values (Keep this as the last step)
                try:
                    intensity_values = (intensity_values - min_value) * (
                        255 / (max_value - min_value)
                    )
                except Exception as e:
                    print(e)
                    pass

                # Update the left and right widgets
                self.sensor_feed.widget.setImage(gray)
                self.analyser.setLuminosityScope(intensity_values)
                # Wait for a short time to avoid overloading the CPU
                self.stop_event.wait(0.01)

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
        self.contrast = QSlider(Qt.Horizontal)
        self.gamma = QSlider(Qt.Horizontal)
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

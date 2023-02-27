import imageio
import threading
import numpy as np

from PyQt5.QtWidgets import QWidget, QSizePolicy
from PyQt5.QtGui import QPainter, QImage, QPixmap, QTransform

SIZE = [640, 480]


# Define the webcam thread to capture frames from the webcam and update the widgets
class WebcamThread(threading.Thread):
    def __init__(self, parent):
        super().__init__()
        self.parent = parent
        self.stop_event = threading.Event()

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
                self.parent.sensor_feed.setImage(gray)
                self.parent.analyser.setLuminosityScope(intensity_values)
                # Wait for a short time to avoid overloading the CPU
                self.stop_event.wait(0.01)

    def stop(self):
        self.stop_event.set()
        self.join()


# Define the left widget to display the grayscale webcam feed
class SensorFeed(QWidget):
    def __init__(self, parent):
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

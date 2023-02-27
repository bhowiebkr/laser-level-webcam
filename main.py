import threading
import subprocess
import imageio
import numpy as np
from PyQt5 import QtWidgets, QtGui, QtCore
from PyQt5.QtWidgets import QSlider, QSizePolicy
from PyQt5.QtCore import Qt
from PyQt5.QtGui import QPainter, QPen

from curves import fit_gaussian


SIZE = [640, 480]
# SIZE = [1920, 1080]


# Define the left widget to display the grayscale webcam feed
class LeftWidget(QtWidgets.QWidget):
    def __init__(self, parent):
        super().__init__(parent)
        self.image = None
        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

    def paintEvent(self, event):
        painter = QtGui.QPainter(self)
        if self.image is not None:
            qimage = QtGui.QImage(
                self.image.data,
                self.image.shape[1],
                self.image.shape[0],
                QtGui.QImage.Format_Grayscale8,
            )
            pixmap = QtGui.QPixmap.fromImage(qimage)
            pixmap = pixmap.transformed(QtGui.QTransform().rotate(-90))
            painter.drawPixmap(self.rect(), pixmap)

    def setImage(self, image):
        self.image = image
        self.update()


# Define the right widget to display the LuminosityScope of luminosity
class RightWidget(QtWidgets.QWidget):
    def __init__(self, parent):
        super().__init__(parent)
        self.LuminosityScope = None
        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

    def paintEvent(self, event):
        painter = QtGui.QPainter(self)
        if self.LuminosityScope is not None:
            # Defind the scope image data as the width (long side) of the image x 256 for pixels
            scopeData = np.zeros((self.LuminosityScope.shape[0], 256)).astype(np.uint8)

            # Loop over intensity values and set scope data
            for i, intensity in enumerate(self.LuminosityScope):
                if np.isnan(intensity):
                    intensity = 0

                scopeData[i, : int(intensity)] = 255

            qimage = QtGui.QImage(
                scopeData,
                scopeData.shape[1],
                scopeData.shape[0],
                QtGui.QImage.Format_Grayscale8,
            )
            pixmap = QtGui.QPixmap.fromImage(qimage)

            # Create a vertical flip transform
            transform = QtGui.QTransform()
            transform.scale(1, -1)
            pixmap = pixmap.transformed(transform)
            painter.drawPixmap(self.rect(), pixmap)

            y_pos = fit_gaussian(
                self.LuminosityScope
            )  # Specify the y position of the line
            if y_pos:
                dataHeight = self.LuminosityScope.shape[0]
                pen = QPen(Qt.green, 4, Qt.SolidLine)
                painter.setPen(pen)
                y_pos = int(
                    self.height()
                    - (y_pos - 0) * (self.height() - 0) / (dataHeight - 0)
                    + 0
                )
                painter.drawLine(0, y_pos, self.width(), y_pos)

    def setLuminosityScope(self, LuminosityScope):
        self.LuminosityScope = LuminosityScope
        self.update()


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
                self.parent.left_widget.setImage(gray)
                self.parent.right_widget.setLuminosityScope(intensity_values)
                # Wait for a short time to avoid overloading the CPU
                self.stop_event.wait(0.01)

    def stop(self):
        self.stop_event.set()
        self.join()


# Define the main window
class MainWindow(QtWidgets.QMainWindow):
    def __init__(self):
        super().__init__()

        self.resize(800, 600)
        self.setWindowTitle(f"Resolution {SIZE[0]}x{SIZE[1]}")

        # Set the main window layout
        central_widget = QtWidgets.QWidget()
        self.setCentralWidget(central_widget)
        self.layout = QtWidgets.QHBoxLayout(central_widget)

        # Layouts
        buttonLayout = QtWidgets.QFormLayout()

        # Create the left and right widgets
        self.left_widget = LeftWidget(self)
        self.right_widget = RightWidget(self)

        # Widgets
        self.smoothingSlider = QSlider(Qt.Horizontal, self)
        self.smoothingSlider.setMinimum(0)
        self.smoothingSlider.setMaximum(100)
        self.smoothingSlider.setValue(0)
        self.smoothingSlider.setTickInterval(1)

        # Add to layouts
        self.layout.addWidget(self.left_widget)
        self.layout.addWidget(self.right_widget)

        # Start the webcam thread
        self.webcam_thread = WebcamThread(self)
        self.webcam_thread.start()

        self.layout.addLayout(buttonLayout)
        buttonLayout.addRow("Smoothing", self.smoothingSlider)


if __name__ == "__main__":
    app = QtWidgets.QApplication([])
    window = MainWindow()
    window.show()
    app.exec_()

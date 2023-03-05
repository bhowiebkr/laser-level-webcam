import numpy as np

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import QWidget, QSizePolicy, QGroupBox, QVBoxLayout, QFormLayout, QSlider, QPushButton
from PySide6.QtGui import QPainter, QImage, QPixmap, QTransform, QPen, QFont

from utils.curves import fit_gaussian
from utils.misc import scale_center_point
import time


# Define the right widget to display the LuminosityScope of luminosity
class AnalyserWidget(QWidget):
    OnZeroPointChanged = Signal(float)
    OnCenterPointChanged = Signal(float)
    OnDataWidthChanged = Signal(float)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.LuminosityScope = None
        self.parent = parent
        self.center_point = None
        self.zero_point = None
        self.sensor_width = None
        self.units = None
        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

    def set_units(self, units):
        self.units = units

    def get_data_width(self):
        return int(self.LuminosityScope.shape[0])

    def reset_zero_point(self):
        self.zero_point = None

    def set_sensor_width(self, width):
        self.sensor_width = width

    def paintEvent(self, event):
        super().paintEvent(event)
        painter = QPainter(self)
        if self.LuminosityScope is not None:
            # Defind the scope image data as the width (long side) of the image x 256 for pixels
            scopeData = np.zeros((self.LuminosityScope.shape[0], 256)).astype(np.uint8)

            # Loop over intensity values and set scope data
            for i, intensity in enumerate(self.LuminosityScope):
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
            pixmap = QPixmap.fromImage(qimage)

            # Create a vertical flip transform
            transform = QTransform()
            transform.scale(1, -1)
            pixmap = pixmap.transformed(transform)
            painter.drawPixmap(self.rect(), pixmap)

            self.center_point = fit_gaussian(self.LuminosityScope)  # Specify the y position of the line
            data_width = self.LuminosityScope.shape[0]
            self.OnCenterPointChanged.emit(self.center_point)

            if self.center_point:
                # Draw the green line

                pen = QPen(Qt.green, 0, Qt.SolidLine)
                painter.setPen(pen)
                y_pos = int(self.height() - self.center_point * self.height() / data_width)
                painter.drawLine(0, y_pos, self.width(), y_pos)

                if self.zero_point:
                    # Draw the text value
                    painter.setFont(QFont("Arial", 12))
                    painter.setPen(Qt.green)
                    center_point_real = scale_center_point(self.sensor_width, data_width, self.center_point, self.zero_point, self.units)
                    text = "{:.2f}".format(center_point_real) + self.units
                    textWidth = painter.fontMetrics().horizontalAdvance(text)
                    textHeight = painter.fontMetrics().height()
                    x = (self.width() - textWidth) / 2
                    y = y_pos - (textHeight / 2)
                    painter.setPen(Qt.green)
                    painter.drawText(int(x), int(y), text)

            if self.zero_point:
                # Draw the zero line
                painter.setPen(Qt.red)
                zero_pos = int(self.height() - self.zero_point * self.height() / data_width)
                painter.drawLine(0, zero_pos, self.width(), zero_pos)

    def set_zero(self, value):
        self.zero_point = value
        self.OnZeroPointChanged.emit(self.zero_point)

    def setLuminosityScope(self, LuminosityScope):
        self.LuminosityScope = LuminosityScope

        # Smoothing
        try:
            # compute the moving average with nearest neighbour
            smoothingFactor = self.parent.smoothing.value()
            kernel = np.ones(2 * smoothingFactor + 1) / (2 * smoothingFactor + 1)
            self.LuminosityScope = np.convolve(self.LuminosityScope, kernel, mode="valid")

            # Find the min and max values
            min_value = np.min(self.LuminosityScope)
            max_value = np.max(self.LuminosityScope)

            # Ensure max_value and min_value are not equal to avoid division by zero
            if max_value == min_value:
                max_value += 1
            # Rescale the intensity values to have a range between 0 and 255
            self.LuminosityScope = (self.LuminosityScope - min_value) * (255 / (max_value - min_value))
            self.OnDataWidthChanged.emit(self.LuminosityScope.shape[0])

        except Exception as e:
            print(e)

        self.update()


class Analyser(QGroupBox):
    def __init__(self, parent=None):
        super().__init__(parent)

        self.setTitle("Analyser")

        # Layouts
        main_layout = QVBoxLayout()
        self.setLayout(main_layout)

        params = QFormLayout()

        # Widgets
        self.widget = AnalyserWidget(self)

        self.smoothing = QSlider(Qt.Horizontal)
        self.smoothing.setMinimum(0)
        self.smoothing.setMaximum(200)
        self.smoothing.setValue(50)
        self.smoothing.setTickInterval(1)

        params.addRow("Smoothness", self.smoothing)

        main_layout.addWidget(self.widget)
        main_layout.addLayout(params)

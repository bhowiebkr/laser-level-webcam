import numpy as np

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QWidget,
    QSizePolicy,
    QGroupBox,
    QVBoxLayout,
    QFormLayout,
    QSlider,
    QPushButton,
)
from PySide6.QtGui import QPainter, QImage, QPixmap, QTransform, QPen, QFont

from utils.curves import fit_gaussian


# Define the right widget to display the LuminosityScope of luminosity
class AnalyserWidget(QWidget):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.LuminosityScope = None
        self.parent = parent
        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

    def cam_updated(self):
        print("cam updated")

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

            y_pos = fit_gaussian(
                self.LuminosityScope
            )  # Specify the y position of the line
            y_pos_float = y_pos
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

            # Draw the value
            painter.setFont(QFont("Arial", 12))
            painter.setPen(Qt.green)
            text = "{:.3f}".format(y_pos_float)
            textWidth = painter.fontMetrics().horizontalAdvance(text)
            textHeight = painter.fontMetrics().height()

            x = (self.width() - textWidth) / 2
            y = y_pos - (textHeight / 2)

            painter.drawText(int(x), int(y), text)

    def setLuminosityScope(self, LuminosityScope):
        self.LuminosityScope = LuminosityScope

        # Smoothing
        try:
            # compute the moving average with nearest neighbour
            smoothingFactor = self.parent.smoothing.value()
            kernel = np.ones(2 * smoothingFactor + 1) / (2 * smoothingFactor + 1)
            self.LuminosityScope = np.convolve(
                self.LuminosityScope, kernel, mode="valid"
            )

            # Find the min and max values
            min_value = np.min(self.LuminosityScope)
            max_value = np.max(self.LuminosityScope)

            # Ensure max_value and min_value are not equal to avoid division by zero
            if max_value == min_value:
                max_value += 1
            # Rescale the intensity values to have a range between 0 and 255
            self.LuminosityScope = (self.LuminosityScope - min_value) * (
                255 / (max_value - min_value)
            )

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
        self.smoothing.setMaximum(100)
        self.smoothing.setValue(0)
        self.smoothing.setTickInterval(1)

        params.addRow("Smoothness", self.smoothing)

        main_layout.addWidget(self.widget)
        main_layout.addLayout(params)

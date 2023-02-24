import sys
import cv2
import numpy as np
from PyQt5 import QtCore, QtGui, QtWidgets
import pyqtgraph as pg


class VideoPlayer(QtWidgets.QWidget):
    def __init__(self):
        super().__init__()

        # initialize the camera
        self.cam = cv2.VideoCapture(0)
        self.width = 1280
        self.height = 720
        self.cam.set(cv2.CAP_PROP_FRAME_WIDTH, self.width)
        self.cam.set(cv2.CAP_PROP_FRAME_HEIGHT, self.height)

        # create the layout for the window
        self.layout = QtWidgets.QHBoxLayout()
        self.setLayout(self.layout)

        # create the widget for displaying the video
        self.video_widget = QtWidgets.QLabel(self)
        self.video_widget.setMinimumSize(QtCore.QSize(self.width, self.height))
        self.video_widget.setMaximumSize(QtCore.QSize(self.width, self.height))
        self.layout.addWidget(self.video_widget)

        # create the widget for displaying the histogram
        self.hist_widget = pg.PlotWidget()
        self.hist_widget.setYRange(0, 1000)
        self.hist_item = pg.PlotCurveItem()
        self.hist_widget.addItem(self.hist_item)
        self.layout.addWidget(self.hist_widget)

        # initialize the histogram data
        self.hist_data = np.zeros((256,))

        # create the timer for updating the video
        self.timer = QtCore.QTimer(self)
        self.timer.timeout.connect(self._update)
        self.timer.start(50)

        # show the window
        self.show()

    def _update(self):
        # read a frame from the camera
        ret, frame = self.cam.read()
        if not ret:
            return

        # convert the frame to grayscale
        gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)

        # calculate the histogram of the grayscale frame
        hist = cv2.calcHist([gray], [0], None, [256], [0, 256])
        self.hist_data = np.squeeze(hist)

        # update the histogram widget
        self.hist_item.setData(self.hist_data)

        # convert the frame to a Qt image
        height, width = gray.shape
        bytes_per_line = width
        qt_image = QtGui.QImage(gray.data, width, height, bytes_per_line, QtGui.QImage.Format_Grayscale8)

        # create a pixmap from the Qt image
        pixmap = QtGui.QPixmap.fromImage(qt_image)

        # scale the pixmap to fit the video widget
        self.video_widget.setPixmap(pixmap.scaled(self.width, self.height, QtCore.Qt.KeepAspectRatio))


if __name__ == '__main__':
    app = QtWidgets.QApplication(sys.argv)
    player = VideoPlayer()
    sys.exit(app.exec_())

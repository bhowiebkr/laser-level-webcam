import threading
import imageio
import numpy as np
from PyQt5 import QtWidgets, QtGui, QtCore

# Define the main window
class MainWindow(QtWidgets.QMainWindow):
    def __init__(self):
        super().__init__()

        # Create the left and right widgets
        self.left_widget = LeftWidget(self)
        self.right_widget = RightWidget(self)

        # Set the main window layout
        central_widget = QtWidgets.QWidget()
        layout = QtWidgets.QHBoxLayout(central_widget)
        layout.addWidget(self.left_widget)
        layout.addWidget(self.right_widget)
        self.setCentralWidget(central_widget)

        # Start the webcam thread
        self.webcam_thread = WebcamThread(self)
        self.webcam_thread.start()

# Define the left widget to display the grayscale webcam feed
class LeftWidget(QtWidgets.QWidget):
    def __init__(self, parent):
        super().__init__(parent)
        self.image = None

    def paintEvent(self, event):
        painter = QtGui.QPainter(self)
        if self.image is not None:
            qimage = QtGui.QImage(self.image.data, self.image.shape[1], self.image.shape[0], QtGui.QImage.Format_Grayscale8)
            pixmap = QtGui.QPixmap.fromImage(qimage)
            scaled_pixmap = pixmap.scaled(self.width(), self.height(), QtCore.Qt.KeepAspectRatio)
            painter.drawPixmap(self.rect(), scaled_pixmap)

    def setImage(self, image):
        self.image = image
        self.update()

# Define the right widget to display the histogram of luminosity
class RightWidget(QtWidgets.QWidget):
    def __init__(self, parent):
        super().__init__(parent)
        self.histogram = None

    def paintEvent(self, event):
        painter = QtGui.QPainter(self)
        if self.histogram is not None:
            painter.setPen(QtCore.Qt.black)
            painter.setBrush(QtGui.QColor(255, 0, 0, 127))
            for i, count in enumerate(self.histogram):
                #painter.drawRect(i, self.height() - count, 1, count)
                painter.drawRect(i, self.height() - int(count), 1, int(count))

    def setHistogram(self, histogram):
        self.histogram = histogram
        self.update()

# Define the webcam thread to capture frames from the webcam and update the widgets
class WebcamThread(threading.Thread):
    def __init__(self, parent):
        super().__init__()
        self.parent = parent
        self.stop_event = threading.Event()

    def run(self):
        with imageio.get_reader('<video0>') as webcam:
            while not self.stop_event.is_set():
                # Read a frame from the webcam
                frame = webcam.get_next_data()

                # Convert the RGB image to grayscale using the luminosity method
                gray = np.dot(frame[...,:3], [0.2126, 0.7152, 0.0722])

                # Compute the histogram of luminosity
                histogram, _ = np.histogram(gray, bins=256)
                # Normalize the histogram
                histogram = histogram / np.max(histogram) * 200
                # Update the left and right widgets
                self.parent.left_widget.setImage(frame)
                self.parent.right_widget.setHistogram(histogram)
                # Wait for a short time to avoid overloading the CPU
                self.stop_event.wait(0.01)

    def stop(self):
        self.stop_event.set()
        self.join()

# Define the main function to create and run the app
def main():
    app = QtWidgets.QApplication([])
    window = MainWindow()
    window.show()
    app.exec_()

if __name__ == '__main__':
    main()

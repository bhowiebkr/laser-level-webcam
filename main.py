import threading
import imageio
import numpy as np
from PyQt5 import QtWidgets, QtGui, QtCore

# Define the main window
class MainWindow(QtWidgets.QMainWindow):
    def __init__(self):
        super().__init__()

        self.resize(800, 600)

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
            pixmap = pixmap.transformed(QtGui.QTransform().rotate(-90))
            pixmap = pixmap.scaled(self.width(), self.height(), QtCore.Qt.KeepAspectRatio)          
            painter.drawPixmap(self.rect(), pixmap)

    def setImage(self, image):
        self.image = image
        self.update()

# Define the right widget to display the LuminosityScope of luminosity
class RightWidget(QtWidgets.QWidget):
    def __init__(self, parent):
        super().__init__(parent)
        self.LuminosityScope = None

    def paintEvent(self, event):
        painter = QtGui.QPainter(self)
        if self.LuminosityScope is not None:
            #painter.setPen(QtCore.Qt.black)
            #painter.setBrush(QtGui.QColor(255, 0, 0, 127))
            #barThickness = int(float(self.height()) / self.LuminosityScope.shape[0])
            #for i, count in enumerate(self.LuminosityScope):
            #    painter.drawRect(0, i, int(count), barThickness)

            # Defind the scope image day as the width (long side) of the image x 256 for pixels 
            scopeData = np.zeros((self.LuminosityScope.shape[0], 256)).astype(np.uint8)

            # Loop over intensity values and set scope data
            for i, intensity in enumerate(self.LuminosityScope):
                scopeData[i, :int(intensity)] = 255

            qimage = QtGui.QImage(scopeData, scopeData.shape[0], scopeData.shape[1], QtGui.QImage.Format_Grayscale8)
            pixmap = QtGui.QPixmap.fromImage(qimage)
            scaled_pixmap = pixmap.scaled(self.width(), self.height(), QtCore.Qt.KeepAspectRatio)          
            painter.drawPixmap(self.rect(), scaled_pixmap)


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
        with imageio.get_reader('<video1>', size=(640, 480)) as webcam:
            while not self.stop_event.is_set():
                # Read a frame from the webcam
                frame = webcam.get_next_data()
                # Convert the RGB image to grayscale using the luminosity method
                gray = np.dot(frame[...,:3], [0.2126, 0.7152, 0.0722]).astype(np.uint8)
                intensity_values = np.mean(gray, axis=0)

                print(intensity_values)

                # Update the left and right widgets
                self.parent.left_widget.setImage(gray)
                self.parent.right_widget.setLuminosityScope(intensity_values)
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

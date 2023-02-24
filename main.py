import sys
import cv2
from PyQt5.QtCore import QTimer
from PyQt5.QtGui import QImage, QPixmap
from PyQt5.QtWidgets import QApplication, QMainWindow, QLabel

class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()

        # Set the window title
        self.setWindowTitle("LaserVision: Real-time Laser Measurement with Webcam")

        # Create a label widget and add it to the window
        self.label = QLabel(self)
        self.label.setGeometry(50, 50, 640, 480)

        # Create a timer to update the video feed
        self.timer = QTimer()
        self.timer.timeout.connect(self.update_frame)
        self.timer.start(30) # 30 ms delay between updates

        # Create a VideoCapture object to read from the webcam
        self.cap = cv2.VideoCapture(0)

    def update_frame(self):
        # Read a frame from the webcam
        ret, frame = self.cap.read()

        # Convert the frame to a QImage
        if ret:
            img = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
            h, w, c = img.shape
            qimg = QImage(img.data, w, h, w * c, QImage.Format_RGB888)
            pixmap = QPixmap.fromImage(qimg)

            # Display the pixmap in the label widget
            self.label.setPixmap(pixmap)

if __name__ == '__main__':
    # Create the QApplication instance
    app = QApplication(sys.argv)

    # Create the main window instance
    window = MainWindow()

    # Show the main window
    window.show()

    # Run the event loop
    sys.exit(app.exec_())

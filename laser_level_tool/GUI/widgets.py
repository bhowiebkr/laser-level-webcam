from PyQt5.QtCore import Qt
from PyQt5.QtGui import QIntValidator
from PyQt5.QtWidgets import QWidget, QHBoxLayout, QLabel, QLineEdit


class ResolutionInputWidget(QWidget):
    def __init__(self, parent=None, width=None, height=None):
        super(ResolutionInputWidget, self).__init__(parent)

        # Create the input fields
        self._widthInput = QLineEdit()
        self._widthInput.setValidator(QIntValidator())  # Only allow integer values
        self._heightInput = QLineEdit()
        self._heightInput.setValidator(QIntValidator())  # Only allow integer values

        if width:
            self._widthInput.setText(str(width))

        if height:
            self._heightInput.setText(str(height))

        # Create the labels
        widthLabel = QLabel("w:")
        heightLabel = QLabel("h:")

        # Create the layout
        layout = QHBoxLayout()
        layout.setContentsMargins(3, 0, 3, 0)
        layout.addWidget(widthLabel)
        layout.addWidget(self._widthInput)
        layout.addWidget(heightLabel)
        layout.addWidget(self._heightInput)
        self.setLayout(layout)

    def getWidth(self):
        return int(self._widthInput.text()) if self._widthInput.text() else 0

    def getHeight(self):
        return int(self._heightInput.text()) if self._heightInput.text() else 0

    def lock(self):
        self._widthInput.setEnabled(False)
        self._heightInput.setEnabled(False)

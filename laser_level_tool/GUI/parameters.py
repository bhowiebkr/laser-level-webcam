from PyQt5 import QtWidgets, QtGui
from PyQt5.QtWidgets import QFormLayout


class ParametersWidget(QtWidgets.QWidget):
    def __init__(self, parent):
        super().__init__(parent)

        form_layout = QFormLayout()
        self.setLayout(form_layout)

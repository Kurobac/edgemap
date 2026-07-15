import signal
import sys

from PyQt6.QtWidgets import QApplication, QMessageBox

from .edgemap_client import EdgemapClient, EdgemapClientError
from .editor import EdgemapEditor


def main() -> int:
    app = QApplication.instance() or QApplication(sys.argv)
    app.setApplicationName("edgemap")
    app.setDesktopFileName("edgemap")
    try:
        client = EdgemapClient.from_environment()
        editor = EdgemapEditor(client.capabilities(), client)
    except (RuntimeError, EdgemapClientError) as error:
        QMessageBox.critical(None, "edgemap", str(error))
        return 1
    editor.show()
    signal.signal(signal.SIGINT, signal.SIG_DFL)
    return app.exec()

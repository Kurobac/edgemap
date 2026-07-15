from PyQt6.QtCore import Qt
from PyQt6.QtWidgets import (
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QVBoxLayout,
)


_DISPLAY_NAMES = {
    "up": "↑",
    "down": "↓",
    "left": "←",
    "right": "→",
    "pageup": "PgUp",
    "pagedown": "PgDn",
    "insert": "Ins",
    "delete": "Del",
    "leftctrl": "Ctrl-L",
    "rightctrl": "Ctrl-R",
    "leftshift": "Shift-L",
    "rightshift": "Shift-R",
    "leftalt": "Alt-L",
    "rightalt": "Alt-R",
    "leftmeta": "Super-L",
    "rightmeta": "Super-R",
    "enter": "Enter",
    "space": "Space",
    "tab": "Tab",
    "esc": "Esc",
    "backspace": "Bksp",
    "capslock": "Caps",
    "kpdot": "KP.",
    "kpenter": "KP↩",
    "kpplus": "KP+",
    "kpminus": "KP-",
    "kp*": "KP*",
    "kpslash": "KP/",
    "numlock": "NumLk",
    "volumeup": "Vol↑",
    "volumedown": "Vol↓",
    "mute": "Mute",
    "playpause": "Play",
    "stop": "Stop",
    "previoussong": "⏮",
    "nextsong": "⏭",
}


def _display_name(name: str) -> str:
    if name in _DISPLAY_NAMES:
        return _DISPLAY_NAMES[name]
    if len(name) == 1 or (name.startswith("f") and name[1:].isdigit()):
        return name.upper()
    if name.startswith("kp") and name[2:].isdigit():
        return name.upper()
    return name.title()


class KeyboardPicker(QDialog):
    def __init__(self, parent, capabilities, current: str | None = None):
        super().__init__(parent)
        self.setWindowTitle("Keyboard Key")
        self.resize(420, 420)
        self._selected = None
        self._key_map: dict[str, str] = {}

        layout = QVBoxLayout(self)
        search = QLineEdit()
        search.setPlaceholderText("Type to filter...")
        layout.addWidget(search)

        self.list_widget = QListWidget()
        self.list_widget.setStyleSheet("QListWidget::item { padding: 4px 8px; }")
        self.list_widget.doubleClicked.connect(self._accept)
        layout.addWidget(self.list_widget)

        for name in capabilities.keyboard_names:
            display = _display_name(name)
            item = QListWidgetItem(f"  {display}  ({name})")
            item.setData(Qt.ItemDataRole.UserRole, name)
            self.list_widget.addItem(item)
            self._key_map[name] = display

        search.textChanged.connect(self._filter)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok
            | QDialogButtonBox.StandardButton.Cancel
        )
        buttons.accepted.connect(self._accept)
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)

        if current and current.startswith("key:"):
            selected_name = current[4:]
            for index in range(self.list_widget.count()):
                item = self.list_widget.item(index)
                if item and item.data(Qt.ItemDataRole.UserRole) == selected_name:
                    self.list_widget.setCurrentRow(index)
                    self.list_widget.scrollToItem(item)
                    break

    def key_name(self) -> str | None:
        return self._selected

    def _filter(self, text: str) -> None:
        query = text.lower()
        for index in range(self.list_widget.count()):
            item = self.list_widget.item(index)
            name = item.data(Qt.ItemDataRole.UserRole) or ""
            display = self._key_map.get(name, "")
            item.setHidden(
                bool(
                    query
                    and query not in name.lower()
                    and query not in display.lower()
                )
            )

    def _accept(self) -> None:
        item = self.list_widget.currentItem() or self.list_widget.item(0)
        if item:
            name = item.data(Qt.ItemDataRole.UserRole)
            if name:
                self._selected = f"key:{name}"
                self.accept()


def pick_keyboard_target(
    parent, combo: QComboBox, previous: str, capabilities
) -> str | None:
    current = previous if previous.startswith("key:") else None
    picker = KeyboardPicker(parent, capabilities, current)
    selected = None
    if picker.exec() == QDialog.DialogCode.Accepted:
        selected = picker.key_name()

    was_blocked = combo.blockSignals(True)
    try:
        combo.setCurrentText(selected or previous)
    finally:
        combo.blockSignals(was_blocked)
    return selected

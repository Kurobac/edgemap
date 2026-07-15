from PyQt6.QtCore import Qt
from PyQt6.QtWidgets import (
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QHBoxLayout,
    QHeaderView,
    QMessageBox,
    QPushButton,
    QTableWidget,
    QVBoxLayout,
    QWidget,
)

from .keyboard import pick_keyboard_target


class ComboDialog(QDialog):
    def __init__(self, parent, name, combos, macros, capabilities):
        super().__init__(parent)
        self.setWindowTitle(f"Combo: {name}")
        self.resize(500, 400)
        self.setFixedSize(500, 400)
        self.name = name
        self.combos = [dict(c) for c in combos] if combos else []
        self.macros = macros
        self.capabilities = capabilities

        self.output_targets = ["Keyboard...", *capabilities.combo_outputs]
        self.output_targets += sorted(macros.keys())

        layout = QVBoxLayout(self)
        table = QTableWidget(0, 3)
        table.setHorizontalHeaderLabels(["Key", "Output", ""])
        table.horizontalHeader().setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        table.horizontalHeader().setSectionResizeMode(1, QHeaderView.ResizeMode.Stretch)
        table.horizontalHeader().setSectionResizeMode(2, QHeaderView.ResizeMode.Fixed)
        table.horizontalHeader().resizeSection(2, 34)
        table.verticalHeader().setVisible(False)
        table.setShowGrid(False)
        table.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        table.horizontalHeader().setSectionsClickable(False)
        layout.addWidget(table)
        self.table = table

        self._rebuild()

        btn_row = QHBoxLayout()
        add = QPushButton("+ Add")
        add.clicked.connect(self._add)
        btn_row.addWidget(add)
        btn_row.addStretch()
        box = QDialogButtonBox(QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel)
        box.accepted.connect(self._save)
        box.rejected.connect(self.reject)
        btn_row.addWidget(box)
        layout.addLayout(btn_row)

    def _save_ui_state(self):
        t = self.table
        for i in range(len(self.combos)):
            kw = t.cellWidget(i, 0)
            ow = t.cellWidget(i, 1)
            if kw and ow:
                self.combos[i]["key"] = kw.currentText()
                self.combos[i]["output"] = ow.currentText()

    def _rebuild(self):
        t = self.table
        t.setRowCount(0)
        t.setRowCount(len(self.combos))
        for i, c in enumerate(self.combos):
            key_combo = QComboBox()
            key_combo.addItems(self.capabilities.combo_keys)
            key_combo.setEditable(True)
            key_combo.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
            key_combo.lineEdit().setStyleSheet("padding-left: 8px; border: none;")
            key_combo.setStyleSheet("QComboBox QAbstractItemView { padding-left: 8px; }")
            key_combo.wheelEvent = lambda e: e.ignore()
            key_combo.setCurrentText(c.get("key", "cross"))
            t.setCellWidget(i, 0, key_combo)
            out_combo = QComboBox()
            out_combo.addItems(self.output_targets)
            out_combo.setEditable(True)
            out_combo.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
            out_combo.lineEdit().setStyleSheet("padding-left: 8px; border: none;")
            out_combo.setStyleSheet("QComboBox QAbstractItemView { padding-left: 8px; }")
            out_combo.wheelEvent = lambda e: e.ignore()
            if c.get("output", "") in self.output_targets or c.get("output", "").startswith("key:"):
                out_combo.setCurrentText(c["output"])
            last_valid_out = out_combo.currentText()
            def make_out_handler(oc, last_valid_ref):
                def handler(text):
                    nonlocal last_valid_ref
                    if text == "Keyboard...":
                        selected = pick_keyboard_target(
                            self, oc, last_valid_ref, self.capabilities
                        )
                        if selected:
                            last_valid_ref = selected
                    else:
                        last_valid_ref = text
                return handler
            handler = make_out_handler(out_combo, last_valid_out)
            out_combo.currentTextChanged.connect(handler)
            t.setCellWidget(i, 1, out_combo)
            rem_btn = QPushButton("✕")
            rem_btn.setFixedSize(24, 18)
            rem_btn.clicked.connect(lambda checked, idx=i: self._remove(idx))
            bw = QWidget()
            bl = QHBoxLayout(bw)
            bl.setContentsMargins(0, 0, 0, 0)
            bl.setAlignment(Qt.AlignmentFlag.AlignCenter)
            bl.addWidget(rem_btn)
            t.setCellWidget(i, 2, bw)

    def _add(self):
        self._save_ui_state()
        self.combos.append({"key": "cross", "output": "circle"})
        self._rebuild()

    def _remove(self, idx):
        self._save_ui_state()
        del self.combos[idx]
        self._rebuild()

    def _save(self):
        # Collect current values from UI widgets
        for i, c in enumerate(self.combos):
            kw = self.table.cellWidget(i, 0)
            ow = self.table.cellWidget(i, 1)
            if kw and ow:
                c["key"] = kw.currentText()
                c["output"] = ow.currentText()
        keys = [c.get("key", "") for c in self.combos if c.get("key")]
        # Capability-backed dropdowns provide the valid values; manually typed
        # input is still checked by Rust validation before the file is saved.
        if self.name in keys:
            QMessageBox.warning(self, "Error", f"Combo key cannot be the same as the modifier button ('{self.name}').")
            return
        if len(keys) != len(set(keys)):
            QMessageBox.warning(self, "Error", "Duplicate combo keys are not allowed.")
            return
        if self.name in ("left_fn", "right_fn"):
            face = {"cross", "circle", "square", "triangle"}
            if any(k in face for k in keys):
                QMessageBox.warning(self, "Error",
                    f"FN+face combos conflict with firmware profile switching and are not supported.")
                return
        self.accept()


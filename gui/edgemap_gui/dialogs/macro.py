from PyQt6.QtCore import Qt
from PyQt6.QtGui import QKeySequence, QShortcut
from PyQt6.QtWidgets import (
    QButtonGroup,
    QComboBox,
    QDialog,
    QHBoxLayout,
    QHeaderView,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMessageBox,
    QPushButton,
    QRadioButton,
    QSpinBox,
    QTableWidget,
    QVBoxLayout,
    QWidget,
)

from ..config_document import find_macro_references, rename_macro
from .keyboard import pick_keyboard_target


class MacroEditor(QDialog):
    """Edit steps for a single macro."""
    def __init__(self, parent, name, macros, capabilities):
        super().__init__(parent)
        self.name = name
        self.macros = macros
        self.capabilities = capabilities
        self.is_new = not name
        display = name or "(new macro)"
        self.setWindowTitle(f"Macro: {display}")
        self.resize(500, 450)
        self.setFixedSize(500, 450)

        md = macros.get(name, {}) if name else {}
        self.mode = md.get("mode", "hold")
        self.steps = [dict(s) for s in md.get("sequence", [])]

        layout = QVBoxLayout(self)

        # Name row
        name_row = QHBoxLayout()
        name_row.addWidget(QLabel("Name:"))
        self.name_edit = QLineEdit(name or "")
        self.name_edit.setPlaceholderText("macro name")
        name_row.addWidget(self.name_edit, 1)
        layout.addLayout(name_row)

        # Mode row
        mode_row = QHBoxLayout()
        mode_row.addWidget(QLabel("Mode:"))
        bg = QButtonGroup(self)
        self.rb_hold = QRadioButton("Hold (loop)")
        self.rb_single = QRadioButton("Single (once)")
        bg.addButton(self.rb_hold); bg.addButton(self.rb_single)
        self.rb_hold.setChecked(self.mode == "hold")
        self.rb_single.setChecked(self.mode == "single")
        mode_row.addWidget(self.rb_hold)
        mode_row.addWidget(self.rb_single)
        mode_row.addStretch()
        layout.addLayout(mode_row)

        table = QTableWidget(0, 4)
        table.setHorizontalHeaderLabels(["Key", "Press", "Release", ""])
        table.horizontalHeader().setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        table.horizontalHeader().setSectionResizeMode(1, QHeaderView.ResizeMode.Stretch)
        table.horizontalHeader().setSectionResizeMode(2, QHeaderView.ResizeMode.Stretch)
        table.horizontalHeader().setSectionResizeMode(3, QHeaderView.ResizeMode.Fixed)
        table.horizontalHeader().resizeSection(3, 34)
        table.verticalHeader().setVisible(False)
        table.setShowGrid(False)
        table.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        table.horizontalHeader().setSectionsClickable(False)
        layout.addWidget(table)
        self.table = table

        self._rebuild()

        btn_row = QHBoxLayout()
        add = QPushButton("+ Add Step")
        add.clicked.connect(self._add)
        add.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        btn_row.addWidget(add)
        btn_row.addStretch()
        self._save_btn = QPushButton("Save")
        self._save_btn.clicked.connect(self._save)
        self._save_btn.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        btn_row.addWidget(self._save_btn)
        layout.addLayout(btn_row)

    def _rebuild(self):
        t = self.table
        t.setRowCount(len(self.steps))
        for i, s in enumerate(self.steps):
            cb = QComboBox()
            cb.addItem("Keyboard...")
            cb.addItems(self.capabilities.macro_step_buttons)
            cb.setEditable(True)
            cb.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
            cb.lineEdit().setStyleSheet("padding-left: 8px; border: none;")
            cb.setStyleSheet("QComboBox QAbstractItemView { padding-left: 8px; }")
            cb.wheelEvent = lambda e: None
            if s.get("key", "").startswith("key:"):
                cb.setCurrentText(s["key"])
            elif s.get("key", "") in self.capabilities.macro_step_buttons:
                cb.setCurrentText(s["key"])
            last_valid_k = cb.currentText()
            def make_k_handler(cbox, last_ref):
                def handler(text):
                    nonlocal last_ref
                    if text == "Keyboard...":
                        selected = pick_keyboard_target(
                            self, cbox, last_ref, self.capabilities
                        )
                        if selected:
                            last_ref = selected
                    else:
                        last_ref = text
                return handler
            handler = make_k_handler(cb, last_valid_k)
            cb.currentTextChanged.connect(handler)
            t.setCellWidget(i, 0, cb)
            ps = QSpinBox(); ps.setRange(0, 99999); ps.setValue(s.get("press_ms", 0))
            ps.wheelEvent = lambda e: None
            t.setCellWidget(i, 1, ps)
            rs = QSpinBox(); rs.setRange(0, 99999); rs.setValue(s.get("release_ms", 0))
            rs.wheelEvent = lambda e: None
            t.setCellWidget(i, 2, rs)
            rem = QPushButton("✕"); rem.setFixedSize(24, 18)
            rem.clicked.connect(lambda checked, idx=i: self._remove(idx))
            rem.setFocusPolicy(Qt.FocusPolicy.NoFocus)
            bw = QWidget()
            bl = QHBoxLayout(bw)
            bl.setContentsMargins(0, 0, 0, 0)
            bl.setAlignment(Qt.AlignmentFlag.AlignCenter)
            bl.addWidget(rem)
            t.setCellWidget(i, 3, bw)

    def _save_ui_state(self):
        t = self.table
        for i, s in enumerate(self.steps):
            kw = t.cellWidget(i, 0)
            ps = t.cellWidget(i, 1)
            rs = t.cellWidget(i, 2)
            if kw and ps and rs:
                s["key"] = kw.currentText()
                s["press_ms"] = ps.value()
                s["release_ms"] = rs.value()

    def _add(self):
        self._save_ui_state()
        self.steps.append({"key": "cross", "press_ms": 0, "release_ms": 200})
        self._rebuild()

    def _remove(self, idx):
        self._save_ui_state()
        del self.steps[idx]
        self._rebuild()

    def _save(self):
        # Collect current values from UI widgets
        for i, s in enumerate(self.steps):
            kw = self.table.cellWidget(i, 0)
            ps = self.table.cellWidget(i, 1)
            rs = self.table.cellWidget(i, 2)
            if kw and ps and rs:
                s["key"] = kw.currentText()
                s["press_ms"] = ps.value()
                s["release_ms"] = rs.value()

        for i, s in enumerate(self.steps):
            if s["release_ms"] <= s["press_ms"]:
                QMessageBox.warning(self, "Error",
                    f"Step {i+1}: release_ms must be greater than press_ms.")
                return

        new_name = self.name_edit.text().strip()
        if not new_name:
            QMessageBox.warning(self, "Error", "Macro name cannot be empty.")
            return
        if new_name != self.name and new_name in self.macros:
            QMessageBox.warning(self, "Error", f"Macro '{new_name}' already exists.")
            return
        if not self.steps:
            QMessageBox.warning(self, "Error", "Macro must have at least one step.")
            return

        # Capability-backed dropdowns provide the valid values; manually typed
        # input is still checked by Rust validation before the file is saved.

        if new_name in self.capabilities.reserved_macro_names:
            QMessageBox.warning(self, "Error",
                f"Macro name '{new_name}' conflicts with a standard button or built-in target.")
            return

        self.name = new_name
        self.mode = "hold" if self.rb_hold.isChecked() else "single"
        self.accept()


class MacroPicker(QDialog):
    """List existing macros; pick one to apply, or create/edit/delete."""
    def __init__(self, parent, config, capabilities, for_button=None):
        super().__init__(parent)
        self.config = config
        self.macros = config.setdefault("macros", {})
        self.for_button = for_button
        self.selected = None
        self.capabilities = capabilities
        title = f"Select Macro{f' for {for_button}' if for_button else ''}"
        self.setWindowTitle(title)
        self.resize(440, 320)
        self.setFixedSize(440, 320)

        layout = QVBoxLayout(self)

        top = QHBoxLayout()
        top.addWidget(QLabel("Macros:"))
        top.addStretch()
        add_btn = QPushButton("+ New")
        add_btn.clicked.connect(self._create_new)
        add_btn.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        top.addWidget(add_btn)
        layout.addLayout(top)

        self.list = QListWidget()
        self.list.itemDoubleClicked.connect(self._edit)
        layout.addWidget(self.list)

        btn_row = QHBoxLayout()
        edit_btn = QPushButton("Edit")
        edit_btn.clicked.connect(self._edit)
        edit_btn.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        edit_btn.setAutoDefault(False)
        btn_row.addWidget(edit_btn)
        del_btn = QPushButton("Delete")
        del_btn.clicked.connect(self._delete)
        del_btn.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        del_btn.setAutoDefault(False)
        btn_row.addWidget(del_btn)
        btn_row.addStretch()

        if for_button:
            apply_btn = QPushButton(f"Apply to {for_button}")
            apply_btn.clicked.connect(self._apply)
            apply_btn.setFocusPolicy(Qt.FocusPolicy.NoFocus)
            apply_btn.setAutoDefault(False)
            btn_row.addWidget(apply_btn)

        layout.addLayout(btn_row)

        self._refresh()

        QShortcut(QKeySequence(Qt.Key.Key_Delete), self).activated.connect(self._delete)

    def _refresh(self):
        self.list.clear()
        for name, md in sorted(self.macros.items()):
            if not name:
                continue
            mode = md.get("mode", "hold")
            n = len(md.get("sequence", []))
            item = QListWidgetItem(f"{name}  —  {n} steps, {mode}")
            item.setData(Qt.ItemDataRole.UserRole, name)
            self.list.addItem(item)

    def _create_new(self):
        dlg = MacroEditor(self, "", self.macros, self.capabilities)
        if dlg.exec() == QDialog.DialogCode.Accepted:
            self.macros[dlg.name] = {"mode": dlg.mode, "sequence": dlg.steps}
            self._refresh()

    def _edit(self):
        item = self.list.currentItem()
        if not item:
            return
        name = item.data(Qt.ItemDataRole.UserRole)
        dlg = MacroEditor(self, name, self.macros, self.capabilities)
        if dlg.exec() == QDialog.DialogCode.Accepted:
            updated = {"mode": dlg.mode, "sequence": dlg.steps}
            if dlg.name != name:
                try:
                    rename_macro(self.config, name, dlg.name)
                except ValueError as e:
                    QMessageBox.warning(self, "Error", str(e))
                    return
                self.macros[dlg.name] = updated
            else:
                self.macros[name] = updated
            self._refresh()

    def _delete(self):
        item = self.list.currentItem()
        if not item:
            return
        name = item.data(Qt.ItemDataRole.UserRole)
        references = find_macro_references(self.config, name)
        if references:
            QMessageBox.warning(
                self,
                "Macro in use",
                f"Cannot delete '{name}'. It is referenced by:\n" + "\n".join(references),
            )
            return
        reply = QMessageBox.question(
            self,
            "Delete macro",
            f"Delete macro '{name}'?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
        )
        if reply != QMessageBox.StandardButton.Yes:
            return
        row = self.list.currentRow()
        del self.macros[name]
        self._refresh()
        new_row = min(row, self.list.count() - 1)
        if new_row >= 0:
            self.list.setCurrentRow(new_row)
        self.list.setFocus()

    def _apply(self):
        item = self.list.currentItem()
        if item:
            self.selected = item.data(Qt.ItemDataRole.UserRole)
            self.accept()


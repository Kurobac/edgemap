#!/usr/bin/env python3
"""edgemap config editor — v6 (two-column Excel style)"""

import os, sys, tomllib, signal

from PyQt6.QtCore import Qt, QTimer
from PyQt6.QtGui import QIcon
from PyQt6.QtWidgets import (
    QApplication, QMainWindow, QTableWidget, QTableWidgetItem,
    QHeaderView, QComboBox, QCheckBox, QSpinBox,
    QPushButton, QLabel, QMenu, QMenuBar, QStatusBar,
    QWidget, QHBoxLayout, QVBoxLayout, QDialog, QDialogButtonBox,
    QRadioButton, QButtonGroup, QListWidget, QListWidgetItem,
    QLineEdit, QMessageBox, QToolBar, QStyle, QSizePolicy, QToolButton,
)

# Left column
LEFT = [
    ("Face Buttons", [
        "cross", "circle", "square", "triangle",
    ]),
    ("Shoulder + Triggers", [
        "l1", "r1", "l2", "r2",
    ]),
    ("Stick", [
        "l3", "r3",
    ]),
]

# Right column
RIGHT = [
    ("System", [
        "create", "options", "ps", "touchpad",
        "touchpad_left", "touchpad_right",
    ]),
    ("D-Pad", [
        "dpad_up", "dpad_down", "dpad_left", "dpad_right",
    ]),
    ("Edge — Paddles & Fn", [
        "left_paddle", "right_paddle", "left_fn", "right_fn",
    ]),
]

TARGETS = [
    "cross", "circle", "square", "triangle",
    "l1", "r1", "l2", "r2", "l3", "r3",
    "options", "create", "ps",
    "dpad_up", "dpad_down", "dpad_left", "dpad_right",
    "touchpad", "block", "combo", "macro",
    "ls_up", "ls_down", "ls_left", "ls_right",
    "rs_up", "rs_down", "rs_left", "rs_right",
    "l2_full", "r2_full",
]

COLUMNS = 3  # Button, Remap, Turbo


def load_config():
    path = os.path.expanduser("~/.config/edgemap/default.toml")
    if not os.path.exists(path):
        return {}
    with open(path, "rb") as f:
        return tomllib.load(f)


class ComboDialog(QDialog):
    def __init__(self, parent, name, combos, macros):
        super().__init__(parent)
        self.setWindowTitle(f"Combo: {name}")
        self.resize(420, 220)
        self.name = name
        self.combos = [dict(c) for c in combos] if combos else []
        self.macros = macros

        # Build output items: TARGETS minus "combo"/"macro" + macro names
        self.output_targets = [t for t in TARGETS if t not in ("combo", "macro")]
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

    def _rebuild(self):
        t = self.table
        t.setRowCount(0)
        t.setRowCount(len(self.combos))
        for i, c in enumerate(self.combos):
            key_combo = QComboBox()
            key_combo.addItems(BUTTON_KEYS)
            key_combo.setEditable(True)
            key_combo.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
            key_combo.lineEdit().setStyleSheet("padding-left: 8px; border: none;")
            key_combo.setStyleSheet("QComboBox QAbstractItemView { padding-left: 8px; }")
            key_combo.wheelEvent = lambda e: None
            key_combo.setCurrentText(c.get("key", "cross"))
            t.setCellWidget(i, 0, key_combo)
            out_combo = QComboBox()
            out_combo.addItems(self.output_targets)
            out_combo.setEditable(True)
            out_combo.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
            out_combo.lineEdit().setStyleSheet("padding-left: 8px; border: none;")
            out_combo.setStyleSheet("QComboBox QAbstractItemView { padding-left: 8px; }")
            out_combo.wheelEvent = lambda e: None
            if c.get("output", "") in self.output_targets:
                out_combo.setCurrentText(c["output"])
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
        self.combos.append({"key": "cross", "output": "circle"})
        self._rebuild()

    def _remove(self, idx):
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
        # NOTE: we intentionally do NOT validate individual key/output names
        # against BUTTON_KEYS / TARGETS here. The dropdown already limits
        # choices to valid values; manually typed garbage will be caught by
        # dseuhid's config::validate() at load time.
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


class MacroEditor(QDialog):
    """Edit steps for a single macro."""
    def __init__(self, parent, name, macros):
        super().__init__(parent)
        self.name = name
        self.is_new = not name
        display = name or "(new macro)"
        self.setWindowTitle(f"Macro: {display}")
        self.resize(520, 310)

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
        table.horizontalHeader().resizeSection(1, 64)
        table.horizontalHeader().resizeSection(2, 64)
        table.horizontalHeader().setSectionResizeMode(3, QHeaderView.ResizeMode.Fixed)
        table.horizontalHeader().resizeSection(3, 34)
        table.verticalHeader().setVisible(False)
        table.setShowGrid(False)
        layout.addWidget(table)
        self.table = table

        self._rebuild()

        btn_row = QHBoxLayout()
        add = QPushButton("+ Add Step")
        add.clicked.connect(self._add)
        btn_row.addWidget(add)
        btn_row.addStretch()
        self._save_btn = QPushButton("Save")
        self._save_btn.clicked.connect(self._save)
        btn_row.addWidget(self._save_btn)
        layout.addLayout(btn_row)

    def _rebuild(self):
        t = self.table
        t.setRowCount(len(self.steps))
        for i, s in enumerate(self.steps):
            cb = QComboBox()
            cb.addItems(BUTTON_KEYS)
            cb.setEditable(True)
            cb.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
            cb.lineEdit().setStyleSheet("padding-left: 8px; border: none;")
            cb.setStyleSheet("QComboBox QAbstractItemView { padding-left: 8px; }")
            cb.wheelEvent = lambda e: None
            if s.get("key", "") in BUTTON_KEYS:
                cb.setCurrentText(s["key"])
            t.setCellWidget(i, 0, cb)
            ps = QSpinBox(); ps.setRange(0, 99999); ps.setValue(s.get("press_ms", 0))
            ps.wheelEvent = lambda e: None
            t.setCellWidget(i, 1, ps)
            rs = QSpinBox(); rs.setRange(0, 99999); rs.setValue(s.get("release_ms", 0))
            rs.wheelEvent = lambda e: None
            t.setCellWidget(i, 2, rs)
            rem = QPushButton("✕"); rem.setFixedSize(24, 18)
            rem.clicked.connect(lambda checked, idx=i: self._remove(idx))
            bw = QWidget()
            bl = QHBoxLayout(bw)
            bl.setContentsMargins(0, 0, 0, 0)
            bl.setAlignment(Qt.AlignmentFlag.AlignCenter)
            bl.addWidget(rem)
            t.setCellWidget(i, 3, bw)

    def _add(self):
        self.steps.append({"key": "cross", "press_ms": 0, "release_ms": 200})
        self._rebuild()

    def _remove(self, idx):
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
        if not self.steps:
            QMessageBox.warning(self, "Error", "Macro must have at least one step.")
            return

        # NOTE: intentional — no per-step key-name validation against
        # BUTTON_KEYS here. Dropdown limits choices; garbage typed manually
        # will be caught by dseuhid config::validate().

        # Macro name conflicts with standard buttons or built-in targets
        conflicts = set(n for _, btns in LEFT + RIGHT for n in btns)
        builtin = {"l2_full", "r2_full", "ls_up", "ls_down", "ls_left", "ls_right",
                   "rs_up", "rs_down", "rs_left", "rs_right", "combo", "macro", "block"}
        if new_name in conflicts or new_name in builtin:
            QMessageBox.warning(self, "Error",
                f"Macro name '{new_name}' conflicts with a standard button or built-in target.")
            return

        self.name = new_name
        self.mode = "hold" if self.rb_hold.isChecked() else "single"
        self.accept()


class MacroPicker(QDialog):
    """List existing macros; pick one to apply, or create/edit/delete."""
    def __init__(self, parent, macros, for_button=None):
        super().__init__(parent)
        self.macros = macros
        self.for_button = for_button
        self.selected = None
        title = f"Select Macro{f' for {for_button}' if for_button else ''}"
        self.setWindowTitle(title)
        self.resize(440, 320)

        layout = QVBoxLayout(self)

        top = QHBoxLayout()
        top.addWidget(QLabel("Macros:"))
        top.addStretch()
        add_btn = QPushButton("+ New")
        add_btn.clicked.connect(self._create_new)
        top.addWidget(add_btn)
        layout.addLayout(top)

        self.list = QListWidget()
        self.list.itemDoubleClicked.connect(self._edit)
        layout.addWidget(self.list)

        btn_row = QHBoxLayout()
        edit_btn = QPushButton("Edit")
        edit_btn.clicked.connect(self._edit)
        btn_row.addWidget(edit_btn)
        del_btn = QPushButton("Delete")
        del_btn.clicked.connect(self._delete)
        btn_row.addWidget(del_btn)
        btn_row.addStretch()

        if for_button:
            apply_btn = QPushButton(f"Apply to {for_button}")
            apply_btn.clicked.connect(self._apply)
            btn_row.addWidget(apply_btn)

        layout.addLayout(btn_row)

        self._refresh()

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
        dlg = MacroEditor(self, "", self.macros)
        if dlg.exec() == QDialog.DialogCode.Accepted and dlg.name and dlg.name not in self.macros:
            self.macros[dlg.name] = {"mode": dlg.mode, "sequence": dlg.steps}
            self._refresh()

    def _edit(self):
        item = self.list.currentItem()
        if not item:
            return
        name = item.data(Qt.ItemDataRole.UserRole)
        dlg = MacroEditor(self, name, self.macros)
        if dlg.exec() == QDialog.DialogCode.Accepted:
            self.macros[name] = {"mode": dlg.mode, "sequence": dlg.steps}
            if dlg.name != name:
                self.macros[dlg.name] = self.macros.pop(name)
            self._refresh()

    def _delete(self):
        item = self.list.currentItem()
        if not item:
            return
        name = item.data(Qt.ItemDataRole.UserRole)
        del self.macros[name]
        self._refresh()

    def _apply(self):
        item = self.list.currentItem()
        if item:
            self.selected = item.data(Qt.ItemDataRole.UserRole)
            self.accept()


# Button keys for combo/macro target selection
BUTTON_KEYS = [
    "cross", "circle", "square", "triangle",
    "l1", "r1", "l2", "r2", "l3", "r3",
    "options", "create", "ps",
    "dpad_up", "dpad_down", "dpad_left", "dpad_right",
    "touchpad",
]


class EdgemapEditor(QMainWindow):
    def __init__(self):
        super().__init__()
        self.config = load_config()
        self.current_file = None
        self._split_rows = {}
        import copy
        self._saved_config = copy.deepcopy(self.config)
        self.setWindowTitle("edgemap Config Editor")
        self.resize(920, 700)
        self.setMinimumSize(900, 600)
        self.setStatusBar(QStatusBar())
        self.statusBar().showMessage("")
        self._build_ui()

    def _build_ui(self):
        # Remove existing toolbar before rebuilding
        if hasattr(self, 'toolbar') and self.toolbar:
            self.removeToolBar(self.toolbar)

        # Toolbar (top)
        tb = QToolBar("Main")
        tb.setMovable(False)
        tb.setToolButtonStyle(Qt.ToolButtonStyle.ToolButtonTextBesideIcon)
        style = QApplication.style()

        # Menu button (Qt standard icon)
        menu = QMenu()
        menu.addAction("Open...", self._open_config)
        menu.addAction("Save As...", self._save_as_config)
        menu.addSeparator()
        menu.addAction("Quit", self.close)
        menu_btn = QToolButton()
        menu_btn.setText("Menu")
        menu_btn.setIcon(QIcon.fromTheme("application-menu"))
        menu_btn.setAutoRaise(True)
        menu_btn.setStyleSheet("QToolButton:hover { background: palette(alternate-base); } QToolButton:pressed { background: palette(alternate-base); }")
        menu_btn.setPalette(tb.palette())
        menu_btn.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        menu_btn.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        menu_btn.setToolButtonStyle(Qt.ToolButtonStyle.ToolButtonTextBesideIcon)
        menu_btn.setMenu(menu)
        tb.addWidget(menu_btn)
        tb.addSeparator()

        act_new = tb.addAction(QIcon.fromTheme("document-new"), "New")
        act_new.triggered.connect(self._new_config)

        act_revert = tb.addAction(style.standardIcon(QStyle.StandardPixmap.SP_DialogResetButton), "Revert")
        act_revert.triggered.connect(self._revert_changes)
        act_reset = tb.addAction(style.standardIcon(QStyle.StandardPixmap.SP_BrowserReload), "Reset")
        act_reset.triggered.connect(self._reset_defaults)

        spacer = QWidget()
        spacer.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Preferred)
        tb.addWidget(spacer)

        act_macros = tb.addAction(style.standardIcon(QStyle.StandardPixmap.SP_FileDialogDetailedView), "Macros")
        QTimer.singleShot(0, lambda: act_macros.triggered.connect(self._open_macros))
        tb.addSeparator()
        tb.addAction(style.standardIcon(QStyle.StandardPixmap.SP_DialogSaveButton), "Save")
        tb.addAction(style.standardIcon(QStyle.StandardPixmap.SP_DialogApplyButton), "Apply")

        self.toolbar = tb
        self.addToolBar(Qt.ToolBarArea.TopToolBarArea, tb)

        c = QWidget()
        self.setCentralWidget(c)
        outer = QVBoxLayout(c)
        outer.setContentsMargins(6, 6, 6, 4)
        outer.setSpacing(4)

        # Two tables side by side
        tables = QHBoxLayout()
        tables.setSpacing(8)
        left = self._make_table(LEFT)
        right = self._make_table(RIGHT)
        tables.addWidget(left)
        tables.addWidget(right)
        outer.addLayout(tables)

        # Profile quick-switch (status bar)
        if hasattr(self, 'profile_btn') and self.profile_btn and self.profile_btn.isVisible():
            # already exists — just refresh the menu
            self.profile_btn.menu().clear()
        else:
            profile_btn = QPushButton("No config")
            profile_btn.setFlat(True)
            profile_btn.setCursor(Qt.CursorShape.PointingHandCursor)
            profile_btn.setFocusPolicy(Qt.FocusPolicy.NoFocus)
            profile_btn.setMinimumWidth(100)
            profile_btn.setMaximumWidth(250)
            self.profile_btn = profile_btn
            self.statusBar().addPermanentWidget(profile_btn)

        profile_menu = self.profile_btn.menu() or QMenu(self.profile_btn)
        edir = os.path.expanduser("~/.config/edgemap")
        if os.path.isdir(edir):
            for fn in sorted(os.listdir(edir)):
                if fn.endswith(".toml") and fn != "edgemap.toml":
                    path = os.path.join(edir, fn)
                    display = fn[:12] + "..." + fn[-16:] if len(fn) > 32 else fn
                    profile_menu.addAction(display, lambda p=path: self._open_config(p))
        self.profile_btn.setMenu(profile_menu)
        profile_menu.aboutToShow.connect(
            lambda: profile_menu.setMinimumWidth(self.profile_btn.width())
        )
    # ── table builder ──

    def _make_table(self, groups):
        rows = sum(len(btns) + 1 for _, btns in groups)  # header + buttons per group
        table = QTableWidget(rows, COLUMNS, self)
        table.setHorizontalHeaderLabels(["Button", "Remap", "Turbo"])
        table.verticalHeader().setVisible(False)
        table.setShowGrid(False)
        table.setAlternatingRowColors(False)
        table.setSelectionMode(QTableWidget.SelectionMode.NoSelection)
        table.setStyleSheet("QTableWidget { background: palette(window); } QTableWidget::item { border-bottom: 1px solid palette(mid); }")

        row = 0
        for group_name, buttons in groups:
            # Header
            lbl = QLabel(group_name)
            lbl.setStyleSheet("font-weight: bold; color: palette(text); background: palette(alternate-base);")
            table.setSpan(row, 0, 1, COLUMNS)
            table.setCellWidget(row, 0, lbl)
            table.setRowHeight(row, 24)
            row += 1

            for name in buttons:
                self._add_row(table, row, name)
                table.setRowHeight(row, 26)
                row += 1

        h = table.horizontalHeader()
        h.setSectionResizeMode(0, h.ResizeMode.Stretch)
        h.setSectionResizeMode(1, h.ResizeMode.Stretch)
        h.setSectionResizeMode(2, h.ResizeMode.Stretch)
        return table

    # ── single row ──

    def _add_row(self, table, row, name):
        btn = self.config.get(name, {})
        cur = btn.get("remap", name) or name

        # touchpad split mode: left/right are disabled unless touchpad=split
        is_split_child = name in ("touchpad_left", "touchpad_right")
        touchpad_remap = self.config.get("touchpad", {}).get("remap", "")
        disabled = is_split_child and touchpad_remap != "split"
        if is_split_child and not btn:
            btn["remap"] = "dpad_left" if name == "touchpad_left" else "dpad_right"
            cur = btn["remap"]

        # Col 0: button name
        lbl = QLabel(name)
        lbl.setStyleSheet("padding-left: 8px; border: none;")
        lbl.setEnabled(not disabled)
        table.setCellWidget(row, 0, lbl)

        # Col 1: remap combo + optional edit link
        remap_widget = QWidget()
        rl = QHBoxLayout(remap_widget)
        rl.setContentsMargins(0, 0, 0, 0)
        rl.setSpacing(2)

        cb = QComboBox()
        # touchpad_left/right cannot use combo mode
        if name in ("touchpad_left", "touchpad_right"):
            cb.addItems([t for t in TARGETS if t not in ("combo", "block")])
        else:
            cb.addItems(TARGETS)
        if name == "touchpad":
            cb.addItem("split")
        # detect old-style macro names and remap to "macro"
        macros = self.config.get("macros", {})
        if cur in macros and cur not in TARGETS:
            cb.setCurrentText("macro")
        elif cur in TARGETS:
            cb.setCurrentText(cur)
        elif name == "touchpad" and cur == "split":
            cb.setCurrentText("split")
        cb.setEditable(True)
        cb.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
        cb.lineEdit().setStyleSheet("padding-left: 8px; border: none;")
        cb.setStyleSheet("QComboBox QAbstractItemView { padding-left: 8px; }")
        cb.wheelEvent = lambda e: None
        rl.addWidget(cb)

        edit_btn = QPushButton("✎")
        edit_btn.setFlat(True)
        edit_btn.setFixedSize(24, 24)
        edit_btn.setCursor(Qt.CursorShape.PointingHandCursor)
        edit_btn.setStyleSheet("QPushButton { border: none; color: palette(link); } QPushButton:disabled { color: palette(mid); } QPushButton:hover:!disabled { color: palette(link-visited); }")
        rl.addWidget(edit_btn)

        remap_widget.setEnabled(not disabled)
        table.setCellWidget(row, 1, remap_widget)

        # Combo / Macro edit trigger
        valid_set = set(TARGETS) | {"split"} | set(self.config.get("macros", {}).keys())
        def on_remap_changed(text):
            try: edit_btn.clicked.disconnect()
            except TypeError: pass

            if text == "combo":
                edit_btn.setEnabled(True)
                edit_btn.clicked.connect(lambda: self._edit_combo(name))
            elif text == "macro":
                edit_btn.setEnabled(True)
                edit_btn.clicked.connect(lambda: self._pick_macro(name))
            else:
                edit_btn.setEnabled(False)
            self.config.setdefault(name, {})["remap"] = text

        def on_edit_finished():
            text = cb.currentText()
            if name in ("touchpad_left", "touchpad_right") and text == "block":
                cb.setCurrentText(init_text)
                return
            if text and text not in valid_set:
                cb.setCurrentText(init_text)

        cb.currentTextChanged.connect(on_remap_changed)
        cb.lineEdit().editingFinished.connect(on_edit_finished)
        # initialize — use current combo text for proper state
        init_text = cb.currentText()
        on_remap_changed(init_text)

        if name == "touchpad":
            def _orig_cb(text):
                on_remap_changed(text)
                enabled = (text == "split")
                for wname in ("touchpad_left", "touchpad_right"):
                    for w in self._split_rows.get(wname, ()):
                        w.setEnabled(enabled)
            # replace the callback
            try: cb.currentTextChanged.disconnect()
            except: pass
            cb.currentTextChanged.connect(_orig_cb)
            _orig_cb(init_text)

        # Col 2: turbo
        tw = QWidget()
        tl = QHBoxLayout(tw)
        tl.setContentsMargins(8, 0, 0, 0)
        tl.setSpacing(0)
        tcb = QCheckBox()
        tcb.setStyleSheet("border: none;")
        tcb.setChecked(btn.get("turbo", False))
        tl.addWidget(tcb)
        tl.addStretch()
        iv = QSpinBox()
        iv.setRange(10, 9999)
        iv.setValue(btn.get("turbo_interval_ms", 100))
        iv.setFixedWidth(60)
        iv.setPrefix("I:")
        iv.setButtonSymbols(QSpinBox.ButtonSymbols.NoButtons)
        iv.setAlignment(Qt.AlignmentFlag.AlignRight)
        iv.setToolTip("Toggle interval (ms)")
        iv.wheelEvent = lambda e: None
        tl.addWidget(iv)
        dv = QSpinBox()
        dv.setRange(0, 9999)
        dv.setValue(btn.get("turbo_delay_ms", 0))
        dv.setFixedWidth(60)
        dv.setPrefix("D:")
        dv.setButtonSymbols(QSpinBox.ButtonSymbols.NoButtons)
        dv.setAlignment(Qt.AlignmentFlag.AlignRight)
        dv.setToolTip("Activation delay (ms)")
        dv.wheelEvent = lambda e: None
        tl.addWidget(dv)
        tw.setEnabled(not disabled)
        table.setCellWidget(row, 2, tw)

        tcb.toggled.connect(lambda checked, n=name: self.config.setdefault(n, {}).update({"turbo": checked}))
        iv.valueChanged.connect(lambda v, n=name: self.config.setdefault(n, {}).update({"turbo_interval_ms": v}))
        dv.valueChanged.connect(lambda v, n=name: self.config.setdefault(n, {}).update({"turbo_delay_ms": v}))

        # Turbo validation: L2/R2 self-map/swap + turbo/macro mutual exclusion
        def _validate_turbo(_=None):
            text = cb.currentText()
            conflict = (name in ("l2", "r2") and text in ("l2", "r2", "")) or text == "macro"
            if conflict:
                tcb.setChecked(False)
            tcb.setEnabled(not conflict)
            iv.setEnabled(not conflict)
            dv.setEnabled(not conflict)
        cb.currentTextChanged.connect(_validate_turbo)
        cb.lineEdit().editingFinished.connect(_validate_turbo)
        _validate_turbo()

        if is_split_child:
            self._split_rows[name] = (lbl, remap_widget, tw)
        # touchpad=split → disable turbo (daemon skips validation in split mode)
        if name == "touchpad":
            def _turbo_split(text):
                tw.setEnabled(text != "split")
            cb.currentTextChanged.connect(_turbo_split)
            _turbo_split(init_text)

    # ── combo / macro editing ──

    def _edit_combo(self, name):
        btn = self.config.setdefault(name, {})
        combos = btn.get("combos", [])
        macros = self.config.get("macros", {})
        dlg = ComboDialog(self, name, combos, macros)
        if dlg.exec() == QDialog.DialogCode.Accepted:
            btn["combos"] = dlg.combos
            btn["remap"] = "combo"

    def _pick_macro(self, name):
        macros = self.config.setdefault("macros", {})
        dlg = MacroPicker(self, macros, for_button=name)
        if dlg.exec() == QDialog.DialogCode.Accepted and dlg.selected:
            btn = self.config.setdefault(name, {})
            btn["remap"] = dlg.selected

    def _open_macros(self):
        macros = self.config.setdefault("macros", {})
        MacroPicker(self, macros, for_button=None).exec()

    def _new_config(self):
        if self.config != self._saved_config:
            reply = QMessageBox.question(self, "New",
                "Discard unsaved changes?",
                QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.Cancel)
            if reply != QMessageBox.StandardButton.Yes:
                return
        self.current_file = None
        defaults = {
            "cross": {"remap": "cross"}, "circle": {"remap": "circle"},
            "square": {"remap": "square"}, "triangle": {"remap": "triangle"},
            "l1": {"remap": "l1"}, "r1": {"remap": "r1"},
            "l2": {"remap": "l2"}, "r2": {"remap": "r2"},
            "l3": {"remap": "l3"}, "r3": {"remap": "r3"},
            "create": {"remap": "create"}, "options": {"remap": "options"},
            "ps": {"remap": "ps"}, "touchpad": {"remap": "touchpad"},
            "touchpad_left": {"remap": "dpad_left"},
            "touchpad_right": {"remap": "dpad_right"},
            "dpad_up": {"remap": "dpad_up"}, "dpad_down": {"remap": "dpad_down"},
            "dpad_left": {"remap": "dpad_left"}, "dpad_right": {"remap": "dpad_right"},
            "left_paddle": {"remap": "l1"}, "right_paddle": {"remap": "r1"},
            "left_fn": {"remap": "create"}, "right_fn": {"remap": "options"},
        }
        self.config = {"version": 2}
        self.config.update(defaults)
        self._split_rows = {}
        import copy
        self._saved_config = copy.deepcopy(self.config)
        self._build_ui()
        self.profile_btn.setText("No config")
        self.statusBar().showMessage("")

    def _open_config(self, path=None):
        if path:
            self.current_file = path
            self.profile_btn.setText(os.path.basename(path))
            conf_dir = os.path.realpath(os.path.expanduser("~/.config/edgemap"))
            if os.path.realpath(path).startswith(conf_dir + os.sep):
                self.statusBar().showMessage("")
            else:
                self.statusBar().showMessage(path)

    def _save_config(self):
        pass

    def _save_as_config(self):
        pass

    def _reset_defaults(self):
        reply = QMessageBox.question(self, "Reset", "Reset all buttons to defaults?",
                                      QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No)
        if reply != QMessageBox.StandardButton.Yes:
            return
        defaults = {
            "cross": {"remap": "cross"}, "circle": {"remap": "circle"},
            "square": {"remap": "square"}, "triangle": {"remap": "triangle"},
            "l1": {"remap": "l1"}, "r1": {"remap": "r1"},
            "l2": {"remap": "l2"}, "r2": {"remap": "r2"},
            "l3": {"remap": "l3"}, "r3": {"remap": "r3"},
            "create": {"remap": "create"}, "options": {"remap": "options"},
            "ps": {"remap": "ps"}, "touchpad": {"remap": "touchpad"},
            "touchpad_left": {"remap": "dpad_left"},
            "touchpad_right": {"remap": "dpad_right"},
            "dpad_up": {"remap": "dpad_up"}, "dpad_down": {"remap": "dpad_down"},
            "dpad_left": {"remap": "dpad_left"}, "dpad_right": {"remap": "dpad_right"},
            "left_paddle": {"remap": "l1"}, "right_paddle": {"remap": "r1"},
            "left_fn": {"remap": "create"}, "right_fn": {"remap": "options"},
        }
        self.config = {"version": 2}
        self.config.update(defaults)
        self._build_ui()

    def _revert_changes(self):
        reply = QMessageBox.question(self, "Revert",
            "Discard all changes since last save?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No)
        if reply != QMessageBox.StandardButton.Yes:
            return
        import copy
        self.config = copy.deepcopy(self._saved_config)
        self._build_ui()


if __name__ == "__main__":
    app = QApplication(sys.argv)
    app.setApplicationName("edgemap")
    w = EdgemapEditor()
    w.show()
    signal.signal(signal.SIGINT, signal.SIG_DFL)
    app.exec()

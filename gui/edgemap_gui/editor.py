#!/usr/bin/env python3
"""edgemap config editor — v6 (two-column Excel style)"""

import os, tomllib
from tomllib import TOMLDecodeError

from .config_document import (
    ConfigDocument,
    atomic_write_text,
    load_profile_config,
)
from .dialogs.combo import ComboDialog
from .dialogs.keyboard import pick_keyboard_target
from .dialogs.macro import MacroPicker
from .dialogs.profiles import EdgemapConfigDialog
from .edgemap_client import EdgemapClientError
from .paths import edgemap_config_dir
from .serializer import default_remap, serialize_config, serialize_profiles

from PyQt6.QtCore import Qt, QTimer
from PyQt6.QtGui import QIcon, QShortcut, QKeySequence
from PyQt6.QtWidgets import (
    QApplication, QMainWindow, QTableWidget,
    QHeaderView, QComboBox, QCheckBox, QSpinBox,
    QPushButton, QLabel, QMenu, QStatusBar,
    QWidget, QHBoxLayout, QVBoxLayout, QDialog,
    QMessageBox, QToolBar, QStyle, QSizePolicy, QFileDialog,
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

COLUMNS = 3  # Button, Remap, Turbo

OUTPUT_DEVICE_LABELS = {
    "auto": "Auto",
    "dualsense": "DualSense",
    "dualshock4": "DualShock 4 (Beta)",
}

OUTPUT_DEVICE_MENU = [
    ("Auto (match physical)", "auto"),
    ("DualSense", "dualsense"),
    ("DualShock 4 (Beta)", "dualshock4"),
]


def load_config():
    path = os.path.join(edgemap_config_dir(), "default.toml")
    if not os.path.exists(path):
        return {}
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except TOMLDecodeError as e:
        raise RuntimeError(f"cannot parse {path}: {e}") from e
    except OSError as e:
        raise RuntimeError(f"cannot read {path}: {e}") from e


class EdgemapEditor(QMainWindow):
    def __init__(self, capabilities, client):
        super().__init__()
        self.capabilities = capabilities
        self.client = client
        self.document = ConfigDocument(load_config())
        self._split_rows = {}
        self.setWindowTitle("edgemap Config Editor")
        self.setWindowIcon(QIcon.fromTheme("edgemap"))
        self.resize(925, 700)
        self.setMinimumSize(900, 600)
        self.setStatusBar(QStatusBar())
        self.statusBar().showMessage("")
        self._build_ui()
        self.document.mark_saved()

        default = os.path.join(edgemap_config_dir(), "default.toml")
        if os.path.exists(default):
            self._open_config(default)

        QShortcut(QKeySequence.StandardKey.Save, self).activated.connect(self._save_config)
        QShortcut(QKeySequence.StandardKey.SaveAs, self).activated.connect(self._save_as_config)
        QShortcut(QKeySequence.StandardKey.Open, self).activated.connect(lambda: self._open_config())
        QShortcut(QKeySequence.StandardKey.New, self).activated.connect(self._new_config)
        QShortcut(QKeySequence.StandardKey.Quit, self).activated.connect(self.close)

    @property
    def config(self):
        return self.document.data

    @config.setter
    def config(self, value):
        if hasattr(self, "document"):
            self.document.data = value
        else:
            self.document = ConfigDocument(value)

    @property
    def current_file(self):
        return self.document.current_file

    @current_file.setter
    def current_file(self, value):
        if hasattr(self, "document"):
            self.document.current_file = value
        else:
            self.document = ConfigDocument(current_file=value)

    def closeEvent(self, event):
        if not self.document.dirty:
            event.accept()
            return
        reply = QMessageBox.warning(self, "Unsaved changes",
            "You have unsaved changes. Save before closing?",
            QMessageBox.StandardButton.Save | QMessageBox.StandardButton.Discard | QMessageBox.StandardButton.Cancel)
        if reply == QMessageBox.StandardButton.Save:
            if self._save_config():
                event.accept()
            else:
                event.ignore()
        elif reply == QMessageBox.StandardButton.Discard:
            event.accept()
        else:
            event.ignore()

    def _build_ui(self):
        # Remove existing toolbar before rebuilding
        if hasattr(self, 'toolbar') and self.toolbar:
            self.removeToolBar(self.toolbar)

        # Toolbar (top)
        tb = QToolBar("Main")
        tb.setMovable(False)
        tb.setToolButtonStyle(Qt.ToolButtonStyle.ToolButtonTextBesideIcon)
        style = QApplication.style()

        act_new = tb.addAction(QIcon.fromTheme("document-new"), "New")
        act_new.triggered.connect(self._new_config)
        act_open = tb.addAction(QIcon.fromTheme("document-open"), "Open")
        act_open.triggered.connect(lambda: self._open_config())
        tb.addSeparator()

        act_revert = tb.addAction(style.standardIcon(QStyle.StandardPixmap.SP_DialogResetButton), "Revert")
        act_revert.triggered.connect(self._revert_changes)
        act_reset = tb.addAction(style.standardIcon(QStyle.StandardPixmap.SP_BrowserReload), "Reset")
        act_reset.triggered.connect(self._reset_defaults)

        spacer = QWidget()
        spacer.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Preferred)
        tb.addWidget(spacer)

        act_macros = tb.addAction(style.standardIcon(QStyle.StandardPixmap.SP_FileDialogDetailedView), "Macros")
        QTimer.singleShot(0, lambda: act_macros.triggered.connect(self._open_macros))
        act_edgemap = tb.addAction(style.standardIcon(QStyle.StandardPixmap.SP_FileDialogContentsView), "edgemap")
        act_edgemap.triggered.connect(self._open_edgemap_config)
        tb.addSeparator()
        act_saveas = tb.addAction(QIcon.fromTheme("document-save-as"), "Save As")
        act_saveas.triggered.connect(self._save_as_config)
        act_save = tb.addAction(style.standardIcon(QStyle.StandardPixmap.SP_DialogSaveButton), "Save")
        act_save.triggered.connect(self._save_config)

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
        known_sources = {name for _, buttons in LEFT + RIGHT for name in buttons}
        left_groups = [
            (group, [name for name in buttons if name in self.capabilities.source_buttons])
            for group, buttons in LEFT
        ]
        right_groups = [
            (group, [name for name in buttons if name in self.capabilities.source_buttons])
            for group, buttons in RIGHT
        ]
        other_sources = [
            name for name in self.capabilities.source_buttons if name not in known_sources
        ]
        if other_sources:
            right_groups.append(("Other", other_sources))
        left = self._make_table(left_groups)
        right = self._make_table(right_groups)
        tables.addWidget(left)
        tables.addWidget(right)
        outer.addLayout(tables)

        # Profile quick-switch (status bar)
        if hasattr(self, 'profile_btn') and self.profile_btn:
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
        edir = edgemap_config_dir()
        if os.path.isdir(edir):
            for fn in sorted(os.listdir(edir)):
                if fn.endswith(".toml") and fn != "edgemap.toml":
                    path = os.path.join(edir, fn)
                    display = fn[:12] + "..." + fn[-16:] if len(fn) > 32 else fn
                    profile_menu.addAction(display, lambda _checked=False, p=path: self._open_config(p))
        self.profile_btn.setMenu(profile_menu)
        profile_menu.aboutToShow.connect(
            lambda: profile_menu.setMinimumWidth(self.profile_btn.width())
        )

        # Output device selector (status bar, left of profile_btn)
        if not hasattr(self, 'device_btn'):
            device_btn = QPushButton("DualSense")
            device_btn.setFlat(True)
            device_btn.setCursor(Qt.CursorShape.PointingHandCursor)
            device_btn.setFocusPolicy(Qt.FocusPolicy.NoFocus)
            device_btn.setMaximumWidth(180)
            self.device_btn = device_btn
            self.statusBar().insertPermanentWidget(0, device_btn)

        dev = self.config.get("output_device", "auto")
        label = OUTPUT_DEVICE_LABELS.get(dev, "Auto")
        self.device_btn.setText(label)

        dev_menu = self.device_btn.menu() or QMenu(self.device_btn)
        dev_menu.clear()
        for text, val in OUTPUT_DEVICE_MENU:
            if val not in self.capabilities.output_devices:
                continue
            dev_menu.addAction(text, lambda _checked=False, v=val: self._set_output_device(v))
        known_devices = {value for _, value in OUTPUT_DEVICE_MENU}
        for value in self.capabilities.output_devices:
            if value not in known_devices:
                dev_menu.addAction(value, lambda _checked=False, v=value: self._set_output_device(v))
        self.device_btn.setMenu(dev_menu)
    def _set_output_device(self, val):
        self.config["output_device"] = val
        label = OUTPUT_DEVICE_LABELS.get(val, "Auto")
        self.device_btn.setText(label)
        if val == "dualshock4":
            QMessageBox.information(
                self,
                "DualShock 4 target",
                "DualShock 4 target is beta. Some native DS4 games under Proton may require the patched Proton build linked in the README.",
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

    @staticmethod
    def _default_remap(name):
        return default_remap(name)

    def _add_row(self, table, row, name):
        btn = self.config.get(name, {})
        default_remap = self._default_remap(name)
        cur = btn.get("remap", default_remap) or default_remap

        # touchpad split mode: left/right are disabled unless touchpad=split
        is_split_child = name in ("touchpad_left", "touchpad_right")
        touchpad_remap = self.config.get("touchpad", {}).get("remap", "")
        disabled = is_split_child and touchpad_remap != "split"
        if is_split_child and not btn:
            btn["remap"] = default_remap
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
        cb.setEditable(True)
        cb.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
        targets = ["Keyboard..."]
        for target in self.capabilities.remap_targets:
            targets.append(target)
            if target == "combo":
                targets.append("macro")
        # touchpad_left/right cannot use combo mode
        if name in ("touchpad_left", "touchpad_right"):
            cb.addItems([target for target in targets if target != "combo"])
        elif name == "touchpad":
            cb.addItem("split")
            cb.addItems(targets)
        else:
            cb.addItems(targets)
        # detect old-style macro names and remap to "macro"
        macros = self.config.get("macros", {})
        if cur in macros and cur not in targets:
            cb.setCurrentText("macro")
        elif cur in targets:
            cb.setCurrentText(cur)
        elif name == "touchpad" and cur == "split":
            cb.setCurrentText("split")
        elif cur.startswith("key:"):
            cb.setCurrentText(cur)
        cb.lineEdit().setStyleSheet("padding-left: 8px; border: none;")
        cb.setStyleSheet("QComboBox QAbstractItemView { padding-left: 8px; }")
        cb.wheelEvent = lambda e: None
        rl.addWidget(cb)

        edit_btn = QPushButton()
        edit_btn.setIcon(QIcon.fromTheme("document-edit"))
        edit_btn.setFlat(True)
        edit_btn.setFixedSize(24, 24)
        edit_btn.setCursor(Qt.CursorShape.PointingHandCursor)
        edit_btn.setStyleSheet("QPushButton { border: none; background: transparent; }")
        rl.addWidget(edit_btn)

        remap_widget.setEnabled(not disabled)
        table.setCellWidget(row, 1, remap_widget)

        # Combo / Macro edit trigger
        valid_set = set(targets) | {"split"} | set(self.config.get("macros", {}).keys())
        last_valid_text = cb.currentText()

        def on_remap_changed(text, writeback=True):
            nonlocal last_valid_text
            try: edit_btn.clicked.disconnect()
            except TypeError: pass

            if text == "Keyboard...":
                edit_btn.hide()
                selected = pick_keyboard_target(
                    self, cb, last_valid_text, self.capabilities
                )
                if selected:
                    self.config.setdefault(name, {})["remap"] = selected
                    last_valid_text = selected
                return

            if text == "combo":
                edit_btn.show()
                edit_btn.clicked.connect(lambda: self._edit_combo(name))
            elif text == "macro":
                edit_btn.show()
                edit_btn.clicked.connect(lambda: self._pick_macro(name))
            elif text == "passthrough":
                edit_btn.hide()
            else:
                edit_btn.hide()
            if writeback:
                self.config.setdefault(name, {})["remap"] = text
            last_valid_text = text

        def on_edit_finished():
            text = cb.currentText()
            if name in ("touchpad_left", "touchpad_right") and text == "block":
                cb.setCurrentText(init_text)
                return
            if text.startswith("key:"):
                return
            if text and text not in valid_set:
                cb.setCurrentText(init_text)

        cb.currentTextChanged.connect(on_remap_changed)
        cb.lineEdit().editingFinished.connect(on_edit_finished)
        # initialize — use current combo text for proper state
        init_text = cb.currentText()
        on_remap_changed(init_text, writeback=False)

        if name == "touchpad":
            def _orig_cb(text, writeback=True):
                on_remap_changed(text, writeback)
                enabled = (text == "split")
                for wname in ("touchpad_left", "touchpad_right"):
                    for w in self._split_rows.get(wname, ()):
                        w.setEnabled(enabled)
            # replace the callback
            try: cb.currentTextChanged.disconnect()
            except TypeError: pass
            cb.currentTextChanged.connect(_orig_cb)
            _orig_cb(init_text, writeback=False)

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
            conflict = (name in ("l2", "r2") and text in ("l2", "r2", "")) or text == "macro" or text == "passthrough"
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
        dlg = ComboDialog(self, name, combos, macros, self.capabilities)
        if dlg.exec() == QDialog.DialogCode.Accepted:
            btn["combos"] = dlg.combos
            btn["remap"] = "combo"

    def _pick_macro(self, name):
        dlg = MacroPicker(self, self.config, self.capabilities, for_button=name)
        if dlg.exec() == QDialog.DialogCode.Accepted and dlg.selected:
            btn = self.config.setdefault(name, {})
            btn["remap"] = dlg.selected

    def _open_macros(self):
        MacroPicker(self, self.config, self.capabilities, for_button=None).exec()

    def _open_edgemap_config(self):
        config_dir = edgemap_config_dir()
        path = os.path.join(config_dir, "edgemap.toml")
        try:
            data = load_profile_config(path)
            config_names = sorted(
                name
                for name in os.listdir(config_dir)
                if name.endswith(".toml") and name != "edgemap.toml"
            ) if os.path.isdir(config_dir) else []
        except (OSError, RuntimeError) as error:
            QMessageBox.warning(self, "Error", str(error))
            return
        dialog = EdgemapConfigDialog(self, data, config_names)
        if dialog.exec() != QDialog.DialogCode.Accepted or dialog.data is None:
            return
        content = serialize_profiles(dialog.data)
        try:
            tomllib.loads(content)
            atomic_write_text(path, content)
        except (OSError, TOMLDecodeError) as error:
            QMessageBox.warning(self, "Error", f"Cannot save {path}: {error}")

    def _new_config(self):
        if self.document.dirty:
            reply = QMessageBox.question(self, "New",
                "Discard unsaved changes?",
                QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.Cancel)
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
        self.document.data = {"version": 2, **defaults}
        self.document.current_file = None
        self._split_rows = {}
        self._build_ui()
        self.document.mark_saved()
        self.profile_btn.setText("No config")
        self.statusBar().showMessage("")

    def _open_config(self, path=None):
        if self.document.dirty:
            reply = QMessageBox.warning(self, "Unsaved changes",
                "You have unsaved changes. Discard and open another config?",
                QMessageBox.StandardButton.Discard | QMessageBox.StandardButton.Cancel)
            if reply != QMessageBox.StandardButton.Discard:
                return

        if path is None:
            path, _ = QFileDialog.getOpenFileName(
                self, "Open config",
                edgemap_config_dir(),
                "TOML files (*.toml);;All files (*)"
            )
            if not path:
                return

        try:
            self.client.validate_path(path)
        except EdgemapClientError as error:
            QMessageBox.warning(self, "Error", str(error))
            return

        # Load
        try:
            with open(path, "rb") as f:
                loaded_config = tomllib.load(f)
        except TOMLDecodeError as e:
            QMessageBox.warning(self, "Error", f"Cannot parse {path}:\n{e}")
            return
        except OSError as e:
            QMessageBox.warning(self, "Error", f"Cannot read {path}:\n{e}")
            return

        self.document.data = loaded_config
        self.document.current_file = path
        self.profile_btn.setText(os.path.basename(path))
        conf_dir = os.path.realpath(edgemap_config_dir())
        if os.path.realpath(path).startswith(conf_dir + os.sep):
            self.statusBar().showMessage("")
        else:
            self.statusBar().showMessage(path)
        self._split_rows = {}
        self._build_ui()
        self.document.mark_saved()

    # ── serialization ──

    def _build_toml(self):
        return serialize_config(self.config, self.capabilities.source_buttons)

    def _validate_content(self, content):
        try:
            self.client.validate_content(content)
        except EdgemapClientError as error:
            QMessageBox.warning(self, "Error", str(error))
            return False
        return True

    def _validate_and_write(self, content, target_path):
        if not self._validate_content(content):
            return False
        try:
            atomic_write_text(target_path, content)
            return True
        except OSError as e:
            QMessageBox.warning(self, "Error", f"Cannot save: {e}")
            return False

    def _save_config(self):
        if not self.current_file:
            return self._save_as_config()
        content = self._build_toml()
        if not self._validate_and_write(content, self.current_file):
            return False
        self.document.mark_saved()
        self.statusBar().showMessage(f"Saved {self.current_file}")
        return True

    def _save_as_config(self):
        content = self._build_toml()
        if not self._validate_content(content):
            return False

        path, _ = QFileDialog.getSaveFileName(
            self, "Save config as",
            edgemap_config_dir(),
            "TOML files (*.toml);;All files (*)"
        )
        if not path:
            return False
        try:
            atomic_write_text(path, content)
        except OSError as e:
            QMessageBox.warning(self, "Error", f"Cannot save: {e}")
            return False
        self.document.mark_saved(path)
        self.profile_btn.setText(os.path.basename(path))
        conf_dir = os.path.realpath(edgemap_config_dir())
        if os.path.realpath(path).startswith(conf_dir + os.sep):
            self.statusBar().showMessage("")
        else:
            self.statusBar().showMessage(path)
        return True

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
        self.document.data = {"version": 2, **defaults}
        self._build_ui()

    def _revert_changes(self):
        reply = QMessageBox.question(self, "Revert",
            "Discard all changes since last save?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No)
        if reply != QMessageBox.StandardButton.Yes:
            return
        self.document.revert()
        self._build_ui()

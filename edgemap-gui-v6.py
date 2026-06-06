#!/usr/bin/env python3
"""edgemap config editor — v6 (two-column Excel style)"""

import os, sys, tomllib, signal, re, subprocess, copy
from tomllib import TOMLDecodeError

from PyQt6.QtCore import Qt, QTimer
from PyQt6.QtGui import QIcon, QShortcut, QKeySequence
from PyQt6.QtWidgets import (
    QApplication, QMainWindow, QTableWidget, QTableWidgetItem,
    QHeaderView, QComboBox, QCheckBox, QSpinBox,
    QPushButton, QLabel, QMenu, QStatusBar,
    QWidget, QHBoxLayout, QVBoxLayout, QDialog, QDialogButtonBox,
    QRadioButton, QButtonGroup, QListWidget, QListWidgetItem,
    QLineEdit, QMessageBox, QToolBar, QStyle, QSizePolicy,
    QFormLayout, QGroupBox, QFileDialog,
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
    # keep in sync with config.rs is_valid_target() + resolve_target()
    "block", "passthrough", "combo", "macro",
    "cross", "circle", "square", "triangle",
    "l1", "r1", "l2", "r2", "l3", "r3",
    "options", "create", "ps",
    "dpad_up", "dpad_down", "dpad_left", "dpad_right",
    "touchpad",
    "ls_up", "ls_down", "ls_left", "ls_right",
    "rs_up", "rs_down", "rs_left", "rs_right",
    "l2_full", "r2_full",
]

COLUMNS = 3  # Button, Remap, Turbo


def load_config():
    path = os.path.expanduser("~/.config/edgemap/default.toml")
    if not os.path.exists(path):
        return {}
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except TOMLDecodeError as e:
        print(f"Warning: cannot parse {path}: {e}", file=sys.stderr)
        return {}
    except OSError as e:
        print(f"Warning: cannot read {path}: {e}", file=sys.stderr)
        return {}


class ComboDialog(QDialog):
    def __init__(self, parent, name, combos, macros):
        super().__init__(parent)
        self.setWindowTitle(f"Combo: {name}")
        self.resize(500, 400)
        self.setFixedSize(500, 400)
        self.name = name
        self.combos = [dict(c) for c in combos] if combos else []
        self.macros = macros

        # Build output items: TARGETS minus "combo"/"macro" + macro names
        self.output_targets = [t for t in TARGETS if t not in ("combo", "macro", "passthrough", "block")]
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
            key_combo.addItems(BUTTON_KEYS)
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
        if not self.steps:
            QMessageBox.warning(self, "Error", "Macro must have at least one step.")
            return

        # NOTE: intentional — no per-step key-name validation against
        # BUTTON_KEYS here. Dropdown limits choices; garbage typed manually
        # will be caught by dseuhid config::validate().

        # Macro name conflicts with standard buttons or built-in targets.
        # keep in sync with config.rs macro-name validation.
        conflicts = set(n for _, btns in LEFT + RIGHT for n in btns)
        builtin = {"l2_full", "r2_full", "ls_up", "ls_down", "ls_left", "ls_right",
                   "rs_up", "rs_down", "rs_left", "rs_right", "combo", "macro", "block",
                   "passthrough", "l2_analog", "r2_analog"}
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
        btn_row.addWidget(edit_btn)
        del_btn = QPushButton("Delete")
        del_btn.clicked.connect(self._delete)
        del_btn.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        btn_row.addWidget(del_btn)
        btn_row.addStretch()

        if for_button:
            apply_btn = QPushButton(f"Apply to {for_button}")
            apply_btn.clicked.connect(self._apply)
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


# Button keys for combo/macro target selection
BUTTON_KEYS = [
    "cross", "circle", "square", "triangle",
    "l1", "r1", "l2", "r2", "l3", "r3",
    "options", "create", "ps",
    "dpad_up", "dpad_down", "dpad_left", "dpad_right",
    "touchpad",
]


class EdgemapConfigDialog(QDialog):
    """Editor for ~/.config/edgemap/edgemap.toml."""
    def __init__(self, parent):
        super().__init__(parent)
        self.setWindowTitle("edgemap.toml Editor")
        self.resize(620, 440)
        self.setFixedSize(620, 440)

        self.path = os.path.expanduser("~/.config/edgemap/edgemap.toml")

        data = {}
        if os.path.exists(self.path):
            try:
                with open(self.path, "rb") as f:
                    raw = tomllib.load(f)
                data["config"] = raw.get("config", "default.toml")
                data["profiles"] = {}
                if "profiles" in raw:
                    for name, val in raw["profiles"].items():
                        data["profiles"][name] = {
                            "config": val.get("config", "default.toml"),
                            "match_process": val.get("match_process", ""),
                            "match_cmdline": val.get("match_cmdline", ""),
                        }
            except TOMLDecodeError as e:
                QMessageBox.warning(self, "Error",
                    f"Cannot parse {self.path}:\n{e}\n\nOpening with minimal config.")
                data["config"] = "default.toml"
                data["profiles"] = {}
            except OSError as e:
                QMessageBox.warning(self, "Error",
                    f"Cannot read {self.path}:\n{e}\n\nOpening with minimal config.")
                data["config"] = "default.toml"
                data["profiles"] = {}
        else:
            data["config"] = "default.toml"
            data["profiles"] = {}

        layout = QVBoxLayout(self)

        # Default config
        fl = QFormLayout()
        self.cfg_combo = QComboBox()
        self.cfg_combo.setFixedWidth(200)
        self._fill_config_combo(self.cfg_combo, data["config"])
        fl.addRow("Default config:", self.cfg_combo)
        layout.addLayout(fl)

        # Profiles
        grp = QGroupBox("Profiles")
        gl = QHBoxLayout(grp)

        self.prof_list = QListWidget()
        self.prof_list.setEditTriggers(QListWidget.EditTrigger.DoubleClicked)
        for name, pcfg in data["profiles"].items():
            item = QListWidgetItem(name)
            item.setData(Qt.ItemDataRole.UserRole, {"config": pcfg["config"],
                "match_process": pcfg.get("match_process", ""),
                "match_cmdline": pcfg.get("match_cmdline", "")})
            self.prof_list.addItem(item)
            item.setFlags(item.flags() | Qt.ItemFlag.ItemIsEditable)
        self.prof_list.currentItemChanged.connect(self._on_select)
        gl.addWidget(self.prof_list, 1)
        pf = QFormLayout()
        self.pf_config = QComboBox()
        self.pf_config.setFixedWidth(200)
        self.pf_process = QLineEdit()
        self.pf_process.setFixedWidth(200)
        self.pf_cmdline = QLineEdit()
        self.pf_cmdline.setFixedWidth(200)
        pf.addRow("Config:", self.pf_config)
        pf.addRow("Match process:", self.pf_process)
        pf.addRow("Match cmdline:", self.pf_cmdline)
        hint = QLabel("Both fields set = AND logic (both must match)")
        hint.setStyleSheet("color: gray; font-size: 11pt; padding-left: 4px;")
        hint.setWordWrap(True)
        pf.addRow("", hint)
        gl.addLayout(pf, 2)

        layout.addWidget(grp)

        # Profile buttons
        pr = QHBoxLayout()
        add_btn = QPushButton("+ Add")
        add_btn.clicked.connect(self._add_profile)
        pr.addWidget(add_btn)
        del_btn = QPushButton("Delete")
        del_btn.clicked.connect(self._del_profile)
        pr.addWidget(del_btn)
        pr.addStretch()
        layout.addLayout(pr)

        # Save / Cancel
        br = QHBoxLayout()
        br.addStretch()
        save_btn = QPushButton("Save")
        save_btn.clicked.connect(self._save)
        br.addWidget(save_btn)
        cancel_btn = QPushButton("Cancel")
        cancel_btn.clicked.connect(self.reject)
        br.addWidget(cancel_btn)
        layout.addLayout(br)

        if self.prof_list.count() > 0:
            self.prof_list.setCurrentRow(0)

    def _save_selection(self, item):
        item.setData(Qt.ItemDataRole.UserRole, {
            "config": self.pf_config.currentText(),
            "match_process": self.pf_process.text(),
            "match_cmdline": self.pf_cmdline.text(),
        })

    def _on_select(self, current, previous):
        if previous:
            self._save_selection(previous)
        if not current:
            return
        d = current.data(Qt.ItemDataRole.UserRole) or {}
        self._fill_config_combo(self.pf_config, d.get("config", "default.toml"))
        self.pf_process.setText(d.get("match_process", ""))
        self.pf_cmdline.setText(d.get("match_cmdline", ""))

    def _add_profile(self):
        item = QListWidgetItem("NewProfile")
        item.setData(Qt.ItemDataRole.UserRole, {"config": "default.toml", "match_process": "", "match_cmdline": ""})
        self.prof_list.addItem(item)
        item.setFlags(item.flags() | Qt.ItemFlag.ItemIsEditable)
        self.prof_list.setCurrentItem(item)

    def _del_profile(self):
        item = self.prof_list.currentItem()
        if not item:
            return
        self.prof_list.takeItem(self.prof_list.currentRow())

    def _fill_config_combo(self, combo, current):
        edir = os.path.expanduser("~/.config/edgemap")
        combo.clear()
        if os.path.isdir(edir):
            for fn in sorted(os.listdir(edir)):
                if fn.endswith(".toml") and fn != "edgemap.toml":
                    combo.addItem(fn)
        combo.setCurrentText(current or "default.toml")

    def _save(self):
        # Save current selection
        cur = self.prof_list.currentItem()
        if cur:
            self._save_selection(cur)

        # Collect names and detect duplicates
        seen = set()
        for i in range(self.prof_list.count()):
            item = self.prof_list.item(i)
            name = item.text().strip()
            if not name:
                QMessageBox.warning(self, "Error", "Profile name cannot be empty.")
                return
            if not re.fullmatch(r'[a-zA-Z0-9_-]+', name):
                QMessageBox.warning(self, "Error",
                    f"Invalid profile name '{name}': only letters, digits, underscores and hyphens allowed.")
                return
            if name in seen:
                QMessageBox.warning(self, "Error", f"Duplicate profile name: '{name}'.")
                return
            seen.add(name)

        # Check match conditions
        for i in range(self.prof_list.count()):
            item = self.prof_list.item(i)
            d = item.data(Qt.ItemDataRole.UserRole) or {}
            if not d.get("match_process") and not d.get("match_cmdline"):
                QMessageBox.warning(self, "Error",
                    f"Profile '{item.text()}' must have at least one match condition.")
                return

        # Check profile configs are not empty
        for i in range(self.prof_list.count()):
            item = self.prof_list.item(i)
            d = item.data(Qt.ItemDataRole.UserRole) or {}
            if not d.get("config", "").strip():
                QMessageBox.warning(self, "Error",
                    f"Profile '{item.text()}': config cannot be empty.")
                return

        # Build TOML
        lines = [f'config = "{self.cfg_combo.currentText()}"']
        if self.prof_list.count() > 0:
            lines.append("")
            for i in range(self.prof_list.count()):
                item = self.prof_list.item(i)
                d = item.data(Qt.ItemDataRole.UserRole) or {}
                lines.append(f"[profiles.{item.text()}]")
                lines.append(f'config = "{d.get("config", "default.toml")}"')
                if d.get("match_process"):
                    lines.append(f'match_process = "{d["match_process"]}"')
                if d.get("match_cmdline"):
                    lines.append(f'match_cmdline = "{d["match_cmdline"]}"')
                lines.append("")

        content = "\n".join(lines) + "\n"
        try:
            os.makedirs(os.path.dirname(self.path), exist_ok=True)
            with open(self.path, "w") as f:
                f.write(content)
        except OSError as e:
            QMessageBox.warning(self, "Error", f"Cannot write {self.path}: {e}")
            return
        self.accept()


class EdgemapEditor(QMainWindow):
    def __init__(self):
        super().__init__()
        self.config = load_config()
        self.current_file = None
        self._split_rows = {}
        self.setWindowTitle("edgemap Config Editor")
        self.setWindowIcon(QIcon(os.path.join(os.path.dirname(__file__), "edgemap.svg")))
        self.resize(920, 700)
        self.setMinimumSize(900, 600)
        self.setStatusBar(QStatusBar())
        self.statusBar().showMessage("")
        self._build_ui()
        self._saved_config = copy.deepcopy(self.config)

        default = os.path.expanduser("~/.config/edgemap/default.toml")
        if os.path.exists(default):
            self._open_config(default)

        QShortcut(QKeySequence.StandardKey.Save, self).activated.connect(self._save_config)
        QShortcut(QKeySequence.StandardKey.SaveAs, self).activated.connect(self._save_as_config)
        QShortcut(QKeySequence.StandardKey.Open, self).activated.connect(lambda: self._open_config())
        QShortcut(QKeySequence.StandardKey.New, self).activated.connect(self._new_config)
        QShortcut(QKeySequence.StandardKey.Quit, self).activated.connect(self.close)

    def closeEvent(self, event):
        if self.config == self._saved_config:
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
        left = self._make_table(LEFT)
        right = self._make_table(RIGHT)
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
            cb.addItems([t for t in TARGETS if t != "combo"])
        elif name == "touchpad":
            cb.addItem("split")
            cb.addItems(TARGETS)
        else:
            cb.addItems(TARGETS)
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
        valid_set = set(TARGETS) | {"split"} | set(self.config.get("macros", {}).keys())
        def on_remap_changed(text):
            try: edit_btn.clicked.disconnect()
            except TypeError: pass

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

    def _open_edgemap_config(self):
        EdgemapConfigDialog(self).exec()

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
        self._build_ui()
        self._saved_config = copy.deepcopy(self.config)
        self.profile_btn.setText("No config")
        self.statusBar().showMessage("")

    def _open_config(self, path=None):
        if self.config != self._saved_config:
            reply = QMessageBox.warning(self, "Unsaved changes",
                "You have unsaved changes. Discard and open another config?",
                QMessageBox.StandardButton.Discard | QMessageBox.StandardButton.Cancel)
            if reply != QMessageBox.StandardButton.Discard:
                return

        if path is None:
            path, _ = QFileDialog.getOpenFileName(
                self, "Open config",
                os.path.expanduser("~/.config/edgemap"),
                "TOML files (*.toml);;All files (*)"
            )
            if not path:
                return

        # Validate via CLI
        try:
            result = subprocess.run(
                ["edgemap", "v", path],
                capture_output=True, text=True, timeout=10
            )
        except FileNotFoundError:
            QMessageBox.warning(self, "Error", "edgemap binary not found in PATH.")
            return
        except subprocess.TimeoutExpired:
            QMessageBox.warning(self, "Error", "Validation timed out.")
            return

        if result.returncode != 0:
            QMessageBox.warning(self, "Error",
                f"Config validation failed:\n{result.stdout}{result.stderr}")
            return

        # Load
        try:
            with open(path, "rb") as f:
                self.config = tomllib.load(f)
        except TOMLDecodeError as e:
            QMessageBox.warning(self, "Error", f"Cannot parse {path}:\n{e}")
            return
        except OSError as e:
            QMessageBox.warning(self, "Error", f"Cannot read {path}:\n{e}")
            return

        self.current_file = path
        self.profile_btn.setText(os.path.basename(path))
        conf_dir = os.path.realpath(os.path.expanduser("~/.config/edgemap"))
        if os.path.realpath(path).startswith(conf_dir + os.sep):
            self.statusBar().showMessage("")
        else:
            self.statusBar().showMessage(path)
        self._split_rows = {}
        self._build_ui()
        self._saved_config = copy.deepcopy(self.config)

    # ── serialization ──

    @staticmethod
    def _escape(val):
        return str(val).replace('\\', '\\\\').replace('"', '\\"')

    def _build_toml(self):
        lines = ["# Generated by edgemap config editor", "version = 2", ""]
        all_btns = [n for _, btns in LEFT + RIGHT for n in btns]
        for name in all_btns:
            if name in ("touchpad_left", "touchpad_right"):
                tp = self.config.get("touchpad", {})
                if tp.get("remap", "") != "split":
                    continue
            btn = self.config.get(name, {})
            remap = btn.get("remap", name) or name
            if remap == "passthrough":
                continue
            if name in ("touchpad_left", "touchpad_right") and not remap:
                remap = "dpad_left" if name == "touchpad_left" else "dpad_right"
            lines.append(f"[{name}]")
            if remap:
                lines.append(f'remap = "{self._escape(remap)}"')
            if btn.get("turbo"):
                lines.append("turbo = true")
                iv = btn.get("turbo_interval_ms", 100)
                if iv != 100:
                    lines.append(f"turbo_interval_ms = {iv}")
                dv = btn.get("turbo_delay_ms", 0)
                if dv:
                    lines.append(f"turbo_delay_ms = {dv}")
            for c in btn.get("combos", []):
                lines.append(f"[[{name}.combos]]")
                lines.append(f'key = "{self._escape(c.get("key",""))}"')
                lines.append(f'output = "{self._escape(c.get("output",""))}"')
            lines.append("")

        macros = self.config.get("macros", {})
        if macros:
            for mname, md in macros.items():
                lines.append(f"[macros.{mname}]")
                lines.append(f'mode = "{self._escape(md.get("mode","hold"))}"')
                for s in md.get("sequence", []):
                    lines.append(f"[[macros.{mname}.sequence]]")
                    lines.append(f'key = "{self._escape(s.get("key",""))}"')
                    lines.append(f"press_ms = {s.get('press_ms', 0)}")
                    lines.append(f"release_ms = {s.get('release_ms', 0)}")
                lines.append("")
        return "\n".join(lines) + "\n"

    def _validate_and_write(self, content, target_path):
        tmp = "/tmp/edgemap_validate.toml"
        try:
            with open(tmp, "w") as f:
                f.write(content)
        except OSError as e:
            QMessageBox.warning(self, "Error", f"Cannot write {tmp}: {e}")
            return False
        try:
            result = subprocess.run(["edgemap", "v", tmp], capture_output=True, text=True, timeout=10)
        except FileNotFoundError:
            QMessageBox.warning(self, "Error", "edgemap binary not found in PATH.")
            return False
        except subprocess.TimeoutExpired:
            QMessageBox.warning(self, "Error", "Validation timed out.")
            return False
        if result.returncode != 0:
            QMessageBox.warning(self, "Error", f"Config validation failed:\n{result.stderr}")
            return False
        try:
            os.makedirs(os.path.dirname(target_path), exist_ok=True)
            with open(target_path, "w") as f:
                f.write(content)
            return True
        except OSError as e:
            QMessageBox.warning(self, "Error", f"Cannot save: {e}")
            return False

    def _save_config(self):
        if not self.current_file:
            self._save_as_config()
            return True
        content = self._build_toml()
        if not self._validate_and_write(content, self.current_file):
            return False
        self._saved_config = copy.deepcopy(self.config)
        self.statusBar().showMessage(f"Saved {self.current_file}")
        return True

    def _save_as_config(self):
        path, _ = QFileDialog.getSaveFileName(
            self, "Save config as",
            os.path.expanduser("~/.config/edgemap"),
            "TOML files (*.toml);;All files (*)"
        )
        if not path:
            return
        content = self._build_toml()
        if not self._validate_and_write(content, path):
            return
        self.current_file = path
        self._saved_config = copy.deepcopy(self.config)
        self.profile_btn.setText(os.path.basename(path))
        conf_dir = os.path.realpath(os.path.expanduser("~/.config/edgemap"))
        if os.path.realpath(path).startswith(conf_dir + os.sep):
            self.statusBar().showMessage("")
        else:
            self.statusBar().showMessage(path)

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
        self.config = copy.deepcopy(self._saved_config)
        self._build_ui()


if __name__ == "__main__":
    app = QApplication(sys.argv)
    app.setApplicationName("edgemap")
    app.setDesktopFileName("edgemap")
    w = EdgemapEditor()
    w.show()
    signal.signal(signal.SIGINT, signal.SIG_DFL)
    app.exec()

#!/usr/bin/env python3
"""edgemap config editor — UI demo v3 (fixed: QTableWidget with parent)"""

import os, sys, tomllib

from PyQt6.QtCore import Qt
from PyQt6.QtWidgets import (
    QApplication, QMainWindow, QTableWidget, QTableWidgetItem,
    QHeaderView, QComboBox, QCheckBox, QSpinBox,
    QPushButton, QLabel, QMenu, QMenuBar, QStatusBar,
    QWidget, QHBoxLayout,
)

BUTTONS = [
    ("cross", "Cross"), ("circle", "Circle"), ("square", "Square"), ("triangle", "Triangle"),
    ("l1", "L1"), ("r1", "R1"), ("l2", "L2"), ("r2", "R2"),
    ("l3", "L3"), ("r3", "R3"),
    ("create", "Create"), ("options", "Options"), ("ps", "PS"),
    ("dpad_up", "DPad Up"), ("dpad_down", "DPad Down"),
    ("dpad_left", "DPad Left"), ("dpad_right", "DPad Right"),
    ("touchpad", "Touchpad"),
    ("left_paddle", "Left Paddle"), ("right_paddle", "Right Paddle"),
    ("left_fn", "Left Fn"), ("right_fn", "Right Fn"),
]

TARGETS = [
    "cross", "circle", "square", "triangle",
    "l1", "r1", "l2", "r2", "l3", "r3",
    "options", "create", "ps",
    "dpad_up", "dpad_down", "dpad_left", "dpad_right",
    "touchpad", "block", "combo",
    "ls_up", "ls_down", "ls_left", "ls_right",
    "rs_up", "rs_down", "rs_left", "rs_right",
    "l2_full", "r2_full",
]


def load_config():
    path = os.path.expanduser("~/.config/edgemap/default.toml")
    if not os.path.exists(path):
        return {}
    with open(path, "rb") as f:
        return tomllib.load(f)


class EdgemapEditor(QMainWindow):
    def __init__(self):
        super().__init__()
        self.config = load_config()
        self.setWindowTitle("edgemap Config Editor (demo)")
        self.resize(760, 600)
        self._build_menus()
        self._build_table()
        self._build_bottom()
        self.setStatusBar(QStatusBar())
        self.statusBar().showMessage("Ready — UI demo only")

    def _build_menus(self):
        mb = QMenuBar(self)
        file = QMenu("&File", self)
        file.addAction("&Open...")
        file.addAction("&Save")
        file.addAction("Save &As...")
        file.addSeparator()
        file.addAction("&Quit", self.close)
        mb.addMenu(file)
        help_menu = QMenu("&Help", self)
        help_menu.addAction("&About")
        mb.addMenu(help_menu)
        self.setMenuBar(mb)

    def _build_table(self):
        # CRITICAL: parent=self prevents layout reparenting deadlock
        table = QTableWidget(len(BUTTONS), 5, self)
        table.setHorizontalHeaderLabels(["Button", "Remap", "Turbo", "Block", "Combo / Macro"])
        table.verticalHeader().setVisible(False)
        table.setShowGrid(False)
        table.setAlternatingRowColors(True)
        table.setSelectionMode(QTableWidget.SelectionMode.NoSelection)

        for i, (name, label) in enumerate(BUTTONS):
            btn_cfg = self.config.get(name, {})
            current_remap = btn_cfg.get("remap", name) or name

            # Column 0 — Button name
            item = QTableWidgetItem(label)
            item.setFlags(Qt.ItemFlag.ItemIsEnabled)
            table.setItem(i, 0, item)

            # Column 1 — Remap dropdown
            combo = QComboBox()
            combo.addItems(TARGETS)
            if current_remap in TARGETS:
                combo.setCurrentText(current_remap)
            else:
                combo.setCurrentIndex(0)
            table.setCellWidget(i, 1, combo)

            # Column 2 — Turbo: checkbox + inline interval/delay
            turbo_widget = QWidget()
            turbo_layout = QHBoxLayout(turbo_widget)
            turbo_layout.setContentsMargins(2, 0, 2, 0)
            turbo_layout.setSpacing(1)
            turbo_cb = QCheckBox()
            turbo_cb.setChecked(btn_cfg.get("turbo", False))
            turbo_layout.addWidget(turbo_cb)
            iv = QSpinBox()
            iv.setRange(10, 1000)
            iv.setValue(btn_cfg.get("turbo_interval_ms", 100))
            iv.setFixedWidth(46)
            iv.setPrefix("i")
            turbo_layout.addWidget(iv)
            dv = QSpinBox()
            dv.setRange(0, 5000)
            dv.setValue(btn_cfg.get("turbo_delay_ms", 0))
            dv.setFixedWidth(46)
            dv.setPrefix("d")
            turbo_layout.addWidget(dv)
            table.setCellWidget(i, 2, turbo_widget)

            # Column 3 — Block
            block_cb = QCheckBox()
            block_cb.setChecked(current_remap == "block")
            block_widget = QWidget()
            block_layout = QHBoxLayout(block_widget)
            block_layout.setContentsMargins(0, 0, 0, 0)
            block_layout.setAlignment(Qt.AlignmentFlag.AlignCenter)
            block_layout.addWidget(block_cb)
            table.setCellWidget(i, 3, block_widget)

            # Column 4 — Combo/Macro summary
            summary = self._summary_for(name, current_remap)
            item = QTableWidgetItem(summary[:35])
            item.setFlags(Qt.ItemFlag.ItemIsEnabled)
            table.setItem(i, 4, item)

        # Column sizing
        header = table.horizontalHeader()
        header.setSectionResizeMode(0, QHeaderView.ResizeMode.ResizeToContents)
        header.setSectionResizeMode(1, QHeaderView.ResizeMode.ResizeToContents)
        header.setSectionResizeMode(2, QHeaderView.ResizeMode.ResizeToContents)
        header.setSectionResizeMode(3, QHeaderView.ResizeMode.Fixed)
        header.resizeSection(3, 60)
        header.setSectionResizeMode(4, QHeaderView.ResizeMode.Stretch)

        self.setCentralWidget(table)

    def _build_bottom(self):
        bar = self.statusBar()
        bar.setSizeGripEnabled(False)
        bar.addWidget(QPushButton("Validate"))
        bar.addWidget(QPushButton("Apply (Reload)"))
        bar.addPermanentWidget(QPushButton("Save"))

    def _summary_for(self, name, remap):
        btn_cfg = self.config.get(name, {})
        if remap == "combo":
            combos = btn_cfg.get("combos", [])
            if combos:
                return ", ".join(f"{c.get('key','?')}→{c.get('output','?')}" for c in combos)
            return ""
        if remap and remap not in TARGETS and remap != "block":
            macros = self.config.get("macros", {})
            if remap in macros:
                md = macros[remap]
                return f"macro: {remap} ({len(md.get('sequence', []))} steps)"
        return ""


if __name__ == "__main__":
    app = QApplication(sys.argv)
    app.setApplicationName("edgemap")
    window = EdgemapEditor()
    window.show()
    app.exec()

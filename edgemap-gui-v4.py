#!/usr/bin/env python3
"""edgemap config editor — UI demo v4 (card layout)"""

import os, sys, tomllib, signal

from PyQt6.QtCore import Qt
from PyQt6.QtGui import QColor
from PyQt6.QtWidgets import (
    QApplication, QMainWindow, QTableWidget, QTableWidgetItem,
    QHeaderView, QComboBox, QCheckBox, QSpinBox,
    QPushButton, QLabel, QMenu, QMenuBar, QStatusBar,
    QWidget, QVBoxLayout, QHBoxLayout,
)

GROUPS = [
    ("Face Buttons", [
        ("cross", "Cross"), ("circle", "Circle"),
        ("square", "Square"), ("triangle", "Triangle"),
    ]),
    ("Shoulder + Triggers", [
        ("l1", "L1"), ("r1", "R1"), ("l2", "L2"), ("r2", "R2"),
    ]),
    ("Stick Buttons", [
        ("l3", "L3"), ("r3", "R3"),
    ]),
    ("System Buttons", [
        ("create", "Create"), ("options", "Options"),
        ("ps", "PS"), ("touchpad", "Touchpad"),
    ]),
    ("D-Pad", [
        ("dpad_up", "DPad Up"), ("dpad_down", "DPad Down"),
        ("dpad_left", "DPad Left"), ("dpad_right", "DPad Right"),
    ]),
    ("Edge — Paddles & Fn", [
        ("left_paddle", "Left Paddle"), ("right_paddle", "Right Paddle"),
        ("left_fn", "Left Fn"), ("right_fn", "Right Fn"),
    ]),
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

COLS = 4


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
        self.resize(880, 680)
        self.setMinimumSize(600, 500)
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
        # 3 rows per group (header + spacer + cards), plus inter-group spacers
        n = len(GROUPS)
        total_rows = n * 3 + (n - 1)
        table = QTableWidget(total_rows, COLS, self)

        # Hide headers, grid, selection
        table.horizontalHeader().setVisible(False)
        table.verticalHeader().setVisible(False)
        table.setShowGrid(False)
        table.setAlternatingRowColors(False)
        table.setSelectionMode(QTableWidget.SelectionMode.NoSelection)
        table.setStyleSheet("QTableWidget { background: palette(window); }")

        row = 0
        for gi, (group_name, buttons) in enumerate(GROUPS):
            # Group header row
            header_item = QTableWidgetItem(group_name)
            header_item.setFlags(Qt.ItemFlag.NoItemFlags)
            font = header_item.font()
            font.setBold(True)
            font.setPointSize(font.pointSize() + 1)
            header_item.setFont(font)
            header_item.setForeground(QColor("#327ac2"))
            header_item.setBackground(table.palette().alternateBase())
            table.setSpan(row, 0, 1, COLS)
            table.setItem(row, 0, header_item)
            table.setRowHeight(row, 24)
            row += 1

            # Spacer between header and cards
            table.setRowHeight(row, 6)
            row += 1

            # Button card row
            for col, (name, btn_label) in enumerate(buttons):
                card = self._make_card(name, btn_label)
                table.setCellWidget(row, col, card)
            while col < COLS - 1:
                dummy = QTableWidgetItem("")
                dummy.setFlags(Qt.ItemFlag.NoItemFlags)
                table.setItem(row, col + 1, dummy)
                col += 1
            table.setRowHeight(row, 68)
            row += 1

            # Inter-group spacer
            if gi < n - 1:
                table.setRowHeight(row, 14)
                row += 1

        # Column sizing — equal width
        for c in range(COLS):
            table.horizontalHeader().setSectionResizeMode(c, QHeaderView.ResizeMode.Stretch)

        self.setCentralWidget(table)

    def _build_bottom(self):
        bar = self.statusBar()
        bar.setSizeGripEnabled(False)
        bar.addWidget(QPushButton("Validate"))
        bar.addWidget(QPushButton("Apply (Reload)"))
        bar.addPermanentWidget(QPushButton("Save"))

    # ── card widget ──

    def _make_card(self, name, btn_label):
        btn_cfg = self.config.get(name, {})
        current_remap = btn_cfg.get("remap", name) or name

        card = QWidget()
        card.setStyleSheet("QWidget { border: 2px solid palette(mid); border-radius: 4px; }")
        layout = QVBoxLayout(card)
        layout.setContentsMargins(4, 4, 4, 4)
        layout.setSpacing(3)

        # Name
        nl = QLabel(btn_label)
        nl.setStyleSheet("font-weight: bold; border: none;")
        nl.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(nl)

        # Remap combo
        combo = QComboBox()
        combo.setStyleSheet("border: 1px solid palette(mid);")
        combo.setMaximumWidth(160)
        combo.addItems(TARGETS)
        if current_remap in TARGETS:
            combo.setCurrentText(current_remap)
        else:
            combo.setCurrentIndex(0)
        layout.addWidget(combo)

        # Turbo row
        tw = QWidget()
        tw.setStyleSheet("border: none;")
        tl = QHBoxLayout(tw)
        tl.setContentsMargins(0, 0, 0, 0)
        tl.setSpacing(1)
        turbo_cb = QCheckBox("T")
        turbo_cb.setStyleSheet("border: none;")
        turbo_cb.setChecked(btn_cfg.get("turbo", False))
        tl.addWidget(turbo_cb)
        iv = QSpinBox()
        iv.setRange(10, 1000)
        iv.setValue(btn_cfg.get("turbo_interval_ms", 100))
        iv.setFixedWidth(56)
        iv.setPrefix("I:")
        tl.addWidget(iv)
        dv = QSpinBox()
        dv.setRange(0, 5000)
        dv.setValue(btn_cfg.get("turbo_delay_ms", 0))
        dv.setFixedWidth(56)
        dv.setPrefix("D:")
        tl.addWidget(dv)
        block_cb = QCheckBox("Block")
        block_cb.setStyleSheet("border: none;")
        block_cb.setChecked(current_remap == "block")
        tl.addWidget(block_cb)
        tl.addStretch()
        layout.addWidget(tw)

        return card

if __name__ == "__main__":
    app = QApplication(sys.argv)
    app.setApplicationName("edgemap")
    window = EdgemapEditor()
    window.show()
    signal.signal(signal.SIGINT, signal.SIG_DFL)
    app.exec()

import re

from PyQt6.QtCore import Qt
from PyQt6.QtWidgets import (
    QComboBox,
    QDialog,
    QFormLayout,
    QGroupBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMessageBox,
    QPushButton,
    QVBoxLayout,
)


class EdgemapConfigDialog(QDialog):
    """Edit a profile model without performing file IO."""

    def __init__(self, parent, data, config_names):
        super().__init__(parent)
        self.setWindowTitle("edgemap.toml Editor")
        self.resize(620, 440)
        self.setFixedSize(620, 440)

        self.config_names = tuple(config_names)
        self.data = None

        layout = QVBoxLayout(self)

        # Default config
        fl = QFormLayout()
        self.cfg_combo = QComboBox()
        self.cfg_combo.setFixedWidth(200)
        self.cfg_combo.setEditable(True)
        self.cfg_combo.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
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
        self.pf_config.setEditable(True)
        self.pf_config.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
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
        combo.clear()
        combo.addItems(self.config_names)
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

        profiles = {}
        for index in range(self.prof_list.count()):
            item = self.prof_list.item(index)
            profiles[item.text()] = item.data(Qt.ItemDataRole.UserRole) or {}
        self.data = {"config": self.cfg_combo.currentText(), "profiles": profiles}
        self.accept()

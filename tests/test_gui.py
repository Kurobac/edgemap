import os
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import patch
import subprocess
import sys


os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "gui"))
import edgemap_gui as package_gui
from edgemap_gui import editor as gui
from edgemap_gui import app as app_module
from edgemap_gui.dialogs import keyboard as keyboard_dialog
from edgemap_gui.dialogs import macro as macro_dialog


class HelperTests(unittest.TestCase):
    def test_app_blocks_startup_when_capabilities_fail(self):
        class FailingClient:
            def capabilities(self):
                raise package_gui.EdgemapClientError("capabilities unavailable")

        with patch.object(
            app_module.EdgemapClient,
            "from_environment",
            return_value=FailingClient(),
        ), patch.object(app_module.QMessageBox, "critical") as critical:
            self.assertEqual(app_module.main(), 1)
        self.assertIn("capabilities unavailable", critical.call_args.args[2])

    def test_capabilities_parse_real_rust_contract(self):
        binary = ROOT / "target" / "debug" / "edgemap"
        output = subprocess.run(
            [str(binary), "capabilities"], text=True, capture_output=True, check=True
        ).stdout
        capabilities = package_gui.Capabilities.from_toml(output)
        self.assertEqual(capabilities.output_devices[0], "auto")
        self.assertIn("touchpad_left", capabilities.source_buttons)
        self.assertEqual(capabilities.keyboard_keys[0].name, "a")
        self.assertEqual(capabilities.keyboard_keys[0].code, 30)

    def test_capabilities_reject_unknown_version(self):
        with self.assertRaisesRegex(ValueError, "unsupported capabilities version"):
            package_gui.Capabilities.from_toml("version = 2\n")

    def test_committed_zipapp_matches_package_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            built = Path(directory) / "edgemap-gui"
            subprocess.run(
                [sys.executable, str(ROOT / "scripts" / "build_gui_zipapp.py"), str(built)],
                check=True,
            )
            self.assertEqual(built.read_bytes(), (ROOT / "edgemap-gui-v6.py").read_bytes())
            imported = subprocess.run(
                [
                    sys.executable,
                    "-c",
                    "import sys; sys.path.insert(0, sys.argv[1]); import edgemap_gui.app",
                    str(built),
                ],
                text=True,
                capture_output=True,
            )
            self.assertEqual(imported.returncode, 0, imported.stderr)

    def test_config_document_tracks_saved_snapshot(self):
        document = package_gui.ConfigDocument({"version": 2})
        self.assertFalse(document.dirty)
        document.data["cross"] = {"remap": "circle"}
        self.assertTrue(document.dirty)
        document.revert()
        self.assertEqual(document.data, {"version": 2})

    def test_package_serializer_matches_legacy_serializer(self):
        config = {
            "version": 2,
            "cross": {"remap": "rapid fire"},
            "macros": {
                "rapid fire": {
                    "mode": "hold",
                    "sequence": [
                        {"key": "key:space", "press_ms": 0, "release_ms": 1}
                    ],
                }
            },
        }
        serialized = package_gui.serialize_config(config, ("cross",))
        parsed = tomllib.loads(serialized)
        self.assertEqual(parsed["cross"]["remap"], "rapid fire")
        self.assertEqual(parsed["macros"]["rapid fire"]["sequence"][0]["key"], "key:space")

    def test_toml_quote_round_trip(self):
        value = 'game "quoted"\\path\nnext'
        parsed = tomllib.loads(f"value = {package_gui.toml_quote(value)}\n")
        self.assertEqual(parsed["value"], value)

    def test_xdg_absolute_path_wins(self):
        env = {"XDG_CONFIG_HOME": "/tmp/xdg"}
        self.assertEqual(gui.edgemap_config_dir(env), "/tmp/xdg/edgemap")

    def test_relative_xdg_falls_back_to_home(self):
        env = {"XDG_CONFIG_HOME": "relative", "HOME": "/home/test"}
        self.assertEqual(gui.edgemap_config_dir(env), "/home/test/.config/edgemap")

    def test_missing_home_rejects_fallback(self):
        with self.assertRaisesRegex(RuntimeError, "HOME"):
            gui.edgemap_config_dir({})

    def test_macro_references_and_rename(self):
        config = {
            "version": 2,
            "cross": {"remap": "burst"},
            "left_fn": {
                "remap": "combo",
                "combos": [{"key": "circle", "output": "burst"}],
            },
            "macros": {"burst": {"mode": "hold", "sequence": []}},
        }
        self.assertEqual(
            package_gui.find_macro_references(config, "burst"),
            ["cross remap", "left_fn combo[circle]"],
        )
        package_gui.rename_macro(config, "burst", "rapid")
        self.assertEqual(config["cross"]["remap"], "rapid")
        self.assertEqual(config["left_fn"]["combos"][0]["output"], "rapid")
        self.assertIn("rapid", config["macros"])
        self.assertNotIn("burst", config["macros"])

    def test_macro_rename_rejects_duplicate(self):
        config = {"macros": {"one": {}, "two": {}}}
        with self.assertRaisesRegex(ValueError, "already exists"):
            package_gui.rename_macro(config, "one", "two")
        self.assertEqual(set(config["macros"]), {"one", "two"})


class WidgetTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = gui.QApplication.instance() or gui.QApplication([])
        binary = ROOT / "target" / "debug" / "edgemap"
        cls.client = package_gui.EdgemapClient(str(binary))
        cls.capabilities = cls.client.capabilities()

    def test_editor_constructs_with_real_capabilities(self):
        with tempfile.TemporaryDirectory() as home, patch.dict(
            os.environ, {"HOME": home}, clear=True
        ):
            editor = gui.EdgemapEditor(self.capabilities, self.client)
            self.assertEqual(editor.windowTitle(), "edgemap Config Editor")
            self.assertEqual(editor.device_btn.text(), "Auto")
            editor.close()

    def test_macro_remap_survives_ui_initialization(self):
        editor = gui.EdgemapEditor.__new__(gui.EdgemapEditor)
        gui.QMainWindow.__init__(editor)
        editor.capabilities = self.capabilities
        editor.config = {
            "version": 2,
            "cross": {"remap": "burst"},
            "macros": {
                "burst": {
                    "mode": "hold",
                    "sequence": [{"key": "circle", "press_ms": 0, "release_ms": 1}],
                }
            },
        }
        editor._split_rows = {}
        table = gui.QTableWidget(1, 3)
        editor._add_row(table, 0, "cross")
        combo = table.cellWidget(0, 1).findChild(gui.QComboBox)
        self.assertEqual(combo.currentText(), "macro")
        self.assertEqual(editor.config["cross"]["remap"], "burst")

    def test_save_config_propagates_save_as_result(self):
        editor = type("Editor", (), {})()
        editor.current_file = None
        editor._save_as_config = lambda: False
        self.assertFalse(gui.EdgemapEditor._save_config(editor))
        editor._save_as_config = lambda: True
        self.assertTrue(gui.EdgemapEditor._save_config(editor))

    def test_save_as_reports_cancel_validation_failure_and_success(self):
        editor = gui.EdgemapEditor.__new__(gui.EdgemapEditor)
        gui.QMainWindow.__init__(editor)
        editor.capabilities = self.capabilities
        editor.setStatusBar(gui.QStatusBar())
        editor.profile_btn = gui.QPushButton()
        editor.config = {"version": 2, "cross": {"remap": "passthrough"}}

        with patch.object(editor, "_validate_content", return_value=False):
            self.assertFalse(editor._save_as_config())
        with patch.object(editor, "_validate_content", return_value=True), patch.object(
            gui.QFileDialog, "getSaveFileName", return_value=("", "")
        ):
            self.assertFalse(editor._save_as_config())
        with tempfile.TemporaryDirectory() as directory, patch.object(
            editor, "_validate_content", return_value=True
        ), patch.object(
            gui.QFileDialog,
            "getSaveFileName",
            return_value=(str(Path(directory) / "saved.toml"), ""),
        ):
            self.assertTrue(editor._save_as_config())
            self.assertTrue((Path(directory) / "saved.toml").exists())
        with patch.object(editor, "_validate_content", return_value=True), patch.object(
            gui.QFileDialog, "getSaveFileName", return_value=("/tmp/fail.toml", "")
        ), patch.object(
            gui, "atomic_write_text", side_effect=OSError("write failed")
        ), patch.object(
            gui.QMessageBox, "warning"
        ):
            self.assertFalse(editor._save_as_config())

    def test_profile_editor_preserves_arbitrary_paths(self):
        with tempfile.TemporaryDirectory() as home, patch.dict(
            os.environ, {"HOME": home}, clear=True
        ):
            config_dir = Path(home) / ".config" / "edgemap"
            config_dir.mkdir(parents=True)
            (config_dir / "local.toml").write_text("version = 2\n")
            (config_dir / "edgemap.toml").write_text(
                'config = "/tmp/default config.toml"\n\n'
                '[profiles.game]\nconfig = "~/profiles/future.toml"\n'
                'match_process = "game"\n'
            )
            data = package_gui.load_profile_config(str(config_dir / "edgemap.toml"))
            dialog = gui.EdgemapConfigDialog(None, data, ["local.toml"])
            self.assertEqual(dialog.cfg_combo.currentText(), "/tmp/default config.toml")
            self.assertEqual(dialog.pf_config.currentText(), "~/profiles/future.toml")

    def test_profile_editor_quotes_special_characters(self):
        with tempfile.TemporaryDirectory() as home, patch.dict(
            os.environ, {"HOME": home}, clear=True
        ):
            config_dir = Path(home) / ".config" / "edgemap"
            config_dir.mkdir(parents=True)
            dialog = gui.EdgemapConfigDialog(
                None,
                {"config": "default.toml", "profiles": {}},
                ["default.toml"],
            )
            dialog._add_profile()
            item = dialog.prof_list.currentItem()
            item.setText("game")
            dialog.pf_cmdline.setText('game "quoted"\\path')
            dialog._save()
            parsed = tomllib.loads(package_gui.serialize_profiles(dialog.data))
            self.assertEqual(parsed["profiles"]["game"]["match_cmdline"], 'game "quoted"\\path')

    def test_profile_validation_failure_does_not_overwrite(self):
        with tempfile.TemporaryDirectory() as home, patch.dict(
            os.environ, {"HOME": home}, clear=True
        ):
            config_dir = Path(home) / ".config" / "edgemap"
            config_dir.mkdir(parents=True)
            path = config_dir / "edgemap.toml"
            original = 'config = "default.toml"\n'
            path.write_text(original)
            editor = gui.EdgemapEditor.__new__(gui.EdgemapEditor)
            gui.QMainWindow.__init__(editor)

            class FakeDialog:
                data = {"config": "default.toml", "profiles": {}}

                def __init__(self, *_args):
                    pass

                def exec(self):
                    return gui.QDialog.DialogCode.Accepted

            with patch.object(gui, "EdgemapConfigDialog", FakeDialog), patch.object(
                gui, "serialize_profiles", return_value='config = "unterminated'
            ), patch.object(gui.QMessageBox, "warning"):
                editor._open_edgemap_config()
            self.assertEqual(path.read_text(), original)

    def test_main_serializer_quotes_macro_table_key(self):
        editor = gui.EdgemapEditor.__new__(gui.EdgemapEditor)
        gui.QMainWindow.__init__(editor)
        editor.capabilities = self.capabilities
        editor.config = {
            "version": 2,
            "cross": {"remap": "rapid fire"},
            "macros": {
                "rapid fire": {
                    "mode": "hold",
                    "sequence": [{"key": "circle", "press_ms": 0, "release_ms": 1}],
                }
            },
        }
        parsed = tomllib.loads(editor._build_toml())
        self.assertIn("rapid fire", parsed["macros"])
        self.assertEqual(parsed["cross"]["remap"], "rapid fire")

    def test_output_device_dualshock4_menu_and_serialization(self):
        editor = gui.EdgemapEditor.__new__(gui.EdgemapEditor)
        gui.QMainWindow.__init__(editor)
        editor.capabilities = self.capabilities
        editor.setStatusBar(gui.QStatusBar())
        editor.config = {"version": 2, "cross": {"remap": "passthrough"}}
        editor._split_rows = {}

        editor._build_ui()
        ds4_actions = [
            action
            for action in editor.device_btn.menu().actions()
            if action.text() == "DualShock 4 (Beta)"
        ]
        self.assertEqual(len(ds4_actions), 1)

        with patch.object(gui.QMessageBox, "information") as info:
            ds4_actions[0].trigger()
        self.assertEqual(editor.config["output_device"], "dualshock4")
        self.assertEqual(editor.device_btn.text(), "DualShock 4 (Beta)")
        info.assert_called_once()
        parsed = tomllib.loads(editor._build_toml())
        self.assertEqual(parsed["output_device"], "dualshock4")

    def test_output_device_dualshock4_existing_config_does_not_warn_on_build(self):
        editor = gui.EdgemapEditor.__new__(gui.EdgemapEditor)
        gui.QMainWindow.__init__(editor)
        editor.capabilities = self.capabilities
        editor.setStatusBar(gui.QStatusBar())
        editor.config = {
            "version": 2,
            "output_device": "dualshock4",
            "cross": {"remap": "passthrough"},
        }
        editor._split_rows = {}

        with patch.object(gui.QMessageBox, "information") as info:
            editor._build_ui()
        self.assertEqual(editor.device_btn.text(), "DualShock 4 (Beta)")
        info.assert_not_called()

    def test_sparse_and_split_configs_serialize_to_valid_rust_config(self):
        editor = gui.EdgemapEditor.__new__(gui.EdgemapEditor)
        gui.QMainWindow.__init__(editor)
        editor.capabilities = self.capabilities
        cases = [
            {"version": 2, "cross": {"remap": "circle"}},
            {"version": 2, "touchpad": {"remap": "split"}},
            {
                "version": 2,
                "cross": {"remap": "rapid fire"},
                "macros": {
                    "rapid fire": {
                        "mode": "hold",
                        "sequence": [{"key": "key:space", "press_ms": 0, "release_ms": 1}],
                    }
                },
            },
        ]
        binary = ROOT / "target" / "debug" / "edgemap"
        self.assertTrue(binary.exists(), "build edgemap before running GUI tests")
        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / "gui-validation.toml"
            for config in cases:
                editor.config = config
                path.write_text(editor._build_toml())
                result = subprocess.run(
                    [str(binary), "validate", str(path)],
                    text=True,
                    capture_output=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_keyboard_picker_initialization_and_writeback(self):
        class FakePicker:
            seen = []
            result = gui.QDialog.DialogCode.Accepted
            selected = "key:a"

            def __init__(self, _parent, _capabilities, current):
                self.seen.append(current)

            def exec(self):
                return self.result

            def key_name(self):
                return self.selected

        editor = gui.EdgemapEditor.__new__(gui.EdgemapEditor)
        gui.QMainWindow.__init__(editor)
        editor.capabilities = self.capabilities
        editor.config = {"version": 2, "cross": {"remap": "key:space"}}
        editor._split_rows = {}
        table = gui.QTableWidget(1, 3)
        with patch.object(keyboard_dialog, "KeyboardPicker", FakePicker):
            editor._add_row(table, 0, "cross")
            combo = table.cellWidget(0, 1).findChild(gui.QComboBox)
            self.assertEqual(combo.currentText(), "key:space")
            self.assertEqual(FakePicker.seen, [])
            combo.setCurrentText("Keyboard...")
        self.assertEqual(combo.currentText(), "key:a")
        self.assertEqual(editor.config["cross"]["remap"], "key:a")
        self.assertEqual(FakePicker.seen, ["key:space"])

    def test_macro_picker_rename_updates_references(self):
        config = {
            "cross": {"remap": "burst"},
            "left_fn": {
                "remap": "combo",
                "combos": [{"key": "circle", "output": "burst"}],
            },
            "macros": {"burst": {"mode": "hold", "sequence": []}},
        }
        picker = gui.MacroPicker(None, config, self.capabilities)
        picker.list.setCurrentRow(0)

        class FakeEditor:
            name = "rapid"
            mode = "single"
            steps = [{"key": "circle", "press_ms": 0, "release_ms": 1}]

            def __init__(self, *_args):
                pass

            def exec(self):
                return gui.QDialog.DialogCode.Accepted

        with patch.object(macro_dialog, "MacroEditor", FakeEditor):
            picker._edit()
        self.assertEqual(config["cross"]["remap"], "rapid")
        self.assertEqual(config["left_fn"]["combos"][0]["output"], "rapid")
        self.assertIn("rapid", config["macros"])

    def test_macro_picker_blocks_referenced_delete(self):
        config = {
            "cross": {"remap": "burst"},
            "macros": {"burst": {"mode": "hold", "sequence": []}},
        }
        picker = gui.MacroPicker(None, config, self.capabilities)
        picker.list.setCurrentRow(0)
        with patch.object(gui.QMessageBox, "warning") as warning:
            picker._delete()
        self.assertIn("burst", config["macros"])
        self.assertIn("cross remap", warning.call_args.args[2])

    def test_macro_picker_confirms_unreferenced_delete(self):
        config = {"macros": {"burst": {"mode": "hold", "sequence": []}}}
        picker = gui.MacroPicker(None, config, self.capabilities)
        picker.list.setCurrentRow(0)
        with patch.object(
            gui.QMessageBox,
            "question",
            return_value=gui.QMessageBox.StandardButton.Yes,
        ):
            picker._delete()
        self.assertNotIn("burst", config["macros"])

    def test_macro_picker_action_buttons_share_style_state(self):
        picker = gui.MacroPicker(
            None, {"macros": {}}, self.capabilities, for_button="cross"
        )
        buttons = {
            button.text(): button
            for button in picker.findChildren(gui.QPushButton)
            if button.text() in ("Edit", "Delete", "Apply to cross")
        }
        self.assertEqual(set(buttons), {"Edit", "Delete", "Apply to cross"})
        for button in buttons.values():
            self.assertEqual(button.focusPolicy(), gui.Qt.FocusPolicy.NoFocus)
            self.assertFalse(button.autoDefault())


if __name__ == "__main__":
    unittest.main()

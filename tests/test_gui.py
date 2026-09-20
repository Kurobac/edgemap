import os
from contextlib import ExitStack
from pathlib import Path
import shutil
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
from edgemap_gui import config_document as config_document_module
from edgemap_gui.dialogs import keyboard as keyboard_dialog
from edgemap_gui.dialogs import macro as macro_dialog
from edgemap_gui.dialogs.combo import ComboDialog
from PyQt6.QtGui import QCloseEvent
from PyQt6.QtWidgets import QLineEdit


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

    def test_installed_launcher_imports_private_package(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory)
            launcher = prefix / "bin" / "edgemap-gui"
            package = prefix / "lib" / "edgemap-gui" / "edgemap_gui"
            launcher.parent.mkdir(parents=True)
            package.parent.mkdir(parents=True)
            shutil.copy2(ROOT / "gui" / "edgemap-gui", launcher)
            shutil.copytree(
                ROOT / "gui" / "edgemap_gui",
                package,
                ignore=shutil.ignore_patterns("__pycache__"),
            )
            self.assertTrue(launcher.stat().st_mode & 0o111)

            imported = subprocess.run(
                [
                    sys.executable,
                    "-c",
                    "from importlib.machinery import SourceFileLoader; "
                    "from importlib.util import module_from_spec, spec_from_loader; "
                    "import sys; "
                    "loader = SourceFileLoader('installed_edgemap_gui', sys.argv[1]); "
                    "spec = spec_from_loader(loader.name, loader); "
                    "module = module_from_spec(spec); "
                    "loader.exec_module(module)",
                    str(launcher),
                ],
                text=True,
                capture_output=True,
            )
            self.assertEqual(imported.returncode, 0, imported.stderr)

    def test_launcher_rejects_missing_private_package(self):
        with tempfile.TemporaryDirectory() as directory:
            launcher = Path(directory) / "bin" / "edgemap-gui"
            launcher.parent.mkdir()
            shutil.copy2(ROOT / "gui" / "edgemap-gui", launcher)
            result = subprocess.run(
                [sys.executable, str(launcher)],
                text=True,
                capture_output=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("installation is incomplete", result.stderr)

    def test_launcher_rejects_old_python_before_package_check(self):
        with tempfile.TemporaryDirectory() as directory:
            launcher = Path(directory) / "bin" / "edgemap-gui"
            launcher.parent.mkdir()
            shutil.copy2(ROOT / "gui" / "edgemap-gui", launcher)
            result = subprocess.run(
                [
                    sys.executable,
                    "-c",
                    "import runpy, sys; "
                    "sys.version_info = (3, 10, 0, 'final', 0); "
                    "runpy.run_path(sys.argv[1], run_name='launcher_test')",
                    str(launcher),
                ],
                text=True,
                capture_output=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("requires Python 3.11 or newer", result.stderr)
            self.assertNotIn("installation is incomplete", result.stderr)

    def test_atomic_write_orders_file_fsync_replace_and_directory_fsync(self):
        events = []
        real_fsync = os.fsync
        real_replace = os.replace

        def record_file_fsync(file_descriptor):
            events.append("file-fsync")
            return real_fsync(file_descriptor)

        def record_replace(source, target):
            events.append("replace")
            return real_replace(source, target)

        with tempfile.TemporaryDirectory() as directory, patch.object(
            config_document_module.os, "fsync", side_effect=record_file_fsync
        ), patch.object(
            config_document_module.os, "replace", side_effect=record_replace
        ), patch.object(
            config_document_module,
            "_fsync_directory",
            side_effect=lambda path: events.append(("directory-fsync", Path(path))),
            create=True,
        ):
            target = Path(directory) / "config.toml"
            package_gui.atomic_write_text(str(target), "version = 2\n")

        self.assertEqual(
            events,
            ["file-fsync", "replace", ("directory-fsync", target.parent)],
        )

    def test_directory_fsync_closes_descriptor(self):
        directory = Path("/tmp/example")
        with patch.object(
            config_document_module.os, "open", return_value=91
        ) as open_directory, patch.object(
            config_document_module.os, "fsync"
        ) as fsync, patch.object(
            config_document_module.os, "close"
        ) as close:
            config_document_module._fsync_directory(directory)

        open_directory.assert_called_once_with(
            directory,
            os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC,
        )
        fsync.assert_called_once_with(91)
        close.assert_called_once_with(91)

    def test_atomic_write_cleans_temporary_file_when_replace_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "config.toml"
            with patch.object(
                config_document_module.os,
                "replace",
                side_effect=OSError("replace failed"),
            ), self.assertRaisesRegex(OSError, "replace failed"):
                package_gui.atomic_write_text(str(target), "version = 2\n")

            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_atomic_write_leaves_no_temporary_file_when_directory_fsync_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "config.toml"
            with patch.object(
                config_document_module,
                "_fsync_directory",
                side_effect=OSError("directory fsync failed"),
                create=True,
            ), self.assertRaisesRegex(OSError, "directory fsync failed"):
                package_gui.atomic_write_text(str(target), "version = 2\n")

            self.assertEqual(target.read_text(encoding="utf-8"), "version = 2\n")
            self.assertEqual(list(Path(directory).iterdir()), [target])

    def test_config_document_tracks_saved_snapshot(self):
        document = package_gui.ConfigDocument({"version": 2})
        self.assertFalse(document.dirty)
        document.data["cross"] = {"remap": "circle"}
        self.assertTrue(document.dirty)
        document.revert()
        self.assertEqual(document.data, {"version": 2})

    def test_serializer_preserves_macro_names_modes_and_steps(self):
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
        self.assertEqual(parsed, config)

    def test_client_reports_process_and_validation_failures(self):
        client = package_gui.EdgemapClient("test-edgemap")
        for error, message in (
            (FileNotFoundError(), "binary not found"),
            (subprocess.TimeoutExpired("test-edgemap", 10), "timed out"),
            (PermissionError("denied"), "failed to run"),
        ):
            with self.subTest(error=error), patch.object(
                subprocess, "run", side_effect=error
            ), self.assertRaisesRegex(package_gui.EdgemapClientError, message):
                client.validate_path("config.toml")
        failed = subprocess.CompletedProcess([], 1, "", "invalid target")
        with patch.object(subprocess, "run", return_value=failed), self.assertRaisesRegex(
            package_gui.EdgemapClientError, "config validation failed: invalid target"
        ):
            client.validate_path("config.toml")

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

    def test_profile_config_rejects_invalid_field_types(self):
        cases = (
            ('config = 1\n', "'config' must be a string"),
            ('profiles = "invalid"\n', "'profiles' must be a table"),
            (
                '[profiles]\ngame = "invalid"\n',
                "profile 'game' must be a table",
            ),
            (
                '[profiles.game]\nconfig = 1\n',
                "profile 'game' field 'config' must be a string",
            ),
            (
                '[profiles.game]\nmatch_process = ["game"]\n',
                "profile 'game' field 'match_process' must be a string",
            ),
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "edgemap.toml"
            for content, message in cases:
                with self.subTest(content=content):
                    path.write_text(content)
                    with self.assertRaisesRegex(RuntimeError, message):
                        package_gui.load_profile_config(str(path))


class WidgetTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = gui.QApplication.instance() or gui.QApplication([])
        binary = ROOT / "target" / "debug" / "edgemap"
        cls.client = package_gui.EdgemapClient(str(binary))
        cls.capabilities = cls.client.capabilities()

    def make_editor(self, directory):
        with patch.dict(os.environ, {"HOME": directory, "XDG_CONFIG_HOME": directory}):
            editor = gui.EdgemapEditor(self.capabilities, self.client)
        self.addCleanup(editor.deleteLater)
        return editor

    def test_existing_file_save_validates_writes_and_advances_saved_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "saved.toml"
            path.write_text('version = 2\n[cross]\nremap = "cross"\n')
            editor = self.make_editor(directory)
            editor.config = tomllib.loads(path.read_text())
            editor.document.mark_saved(str(path))
            editor.config["cross"]["remap"] = "circle"
            self.assertTrue(editor.document.dirty)
            self.assertTrue(editor._save_config())
            self.assertEqual(tomllib.loads(path.read_text())["cross"]["remap"], "circle")
            self.assertFalse(editor.document.dirty)
            self.assertEqual(editor.current_file, str(path))
            editor.config["cross"]["remap"] = "square"
            editor.document.revert()
            self.assertEqual(editor.config["cross"]["remap"], "circle")

    def test_failed_existing_file_save_preserves_bytes_path_and_dirty_state(self):
        for failure in ("validation", "write"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "saved.toml"
                original = 'version = 2\n[cross]\nremap = "cross"\n'
                path.write_text(original)
                editor = self.make_editor(directory)
                editor.config = tomllib.loads(original)
                editor.document.mark_saved(str(path))
                remap = "invalid-target" if failure == "validation" else "circle"
                editor.config["cross"]["remap"] = remap
                with ExitStack() as stack:
                    warning = stack.enter_context(patch.object(gui.QMessageBox, "warning"))
                    if failure == "write":
                        stack.enter_context(patch.object(
                            config_document_module.os, "replace", side_effect=OSError("write failed")
                        ))
                    self.assertFalse(editor._save_config())
                    warning.assert_called_once()
                self.assertEqual(path.read_text(), original)
                self.assertEqual(list(Path(directory).glob("tmp*")), [])
                self.assertEqual(editor.current_file, str(path))
                self.assertTrue(editor.document.dirty)
                self.assertEqual(editor.config["cross"]["remap"], remap)
                editor.document.revert()
                self.assertEqual(editor.config, tomllib.loads(original))

    def test_close_save_keeps_window_open_on_cancel_validation_or_write_failure(self):
        for failure in ("cancel", "validation", "write"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                editor = self.make_editor(directory)
                editor.config = {"version": 2, "cross": {"remap": "cross"}}
                editor.document.mark_saved()
                editor.config["cross"]["remap"] = "invalid-target" if failure == "validation" else "circle"
                target = Path(directory) / "new.toml"
                event = QCloseEvent()
                with ExitStack() as stack:
                    stack.enter_context(patch.object(
                        gui.QMessageBox, "warning", return_value=gui.QMessageBox.StandardButton.Save
                    ))
                    stack.enter_context(patch.object(
                        gui.QFileDialog, "getSaveFileName",
                        return_value=("" if failure == "cancel" else str(target), ""),
                    ))
                    if failure == "write":
                        stack.enter_context(patch.object(
                            config_document_module.os, "replace", side_effect=OSError("write failed")
                        ))
                    editor.closeEvent(event)
                self.assertFalse(event.isAccepted())
                self.assertTrue(editor.document.dirty)
                self.assertIsNone(editor.current_file)
                self.assertFalse(target.exists())

    def test_close_successful_save_writes_file_before_accepting_close(self):
        with tempfile.TemporaryDirectory() as directory:
            editor = self.make_editor(directory)
            editor.config = {"version": 2, "cross": {"remap": "circle"}}
            target = Path(directory) / "new.toml"
            event = QCloseEvent()
            with patch.object(
                gui.QMessageBox, "warning", return_value=gui.QMessageBox.StandardButton.Save
            ), patch.object(gui.QFileDialog, "getSaveFileName", return_value=(str(target), "")):
                editor.closeEvent(event)
            self.assertTrue(event.isAccepted())
            self.assertFalse(editor.document.dirty)
            self.assertEqual(editor.current_file, str(target))
            self.assertEqual(tomllib.loads(target.read_text())["cross"]["remap"], "circle")

    def test_open_invalid_config_preserves_current_document(self):
        with tempfile.TemporaryDirectory() as directory:
            editor = self.make_editor(directory)
            original = {"version": 2, "cross": {"remap": "cross"}}
            editor.config = original.copy()
            editor.document.mark_saved(str(Path(directory) / "original.toml"))
            editor.config = {"version": 2, "cross": {"remap": "circle"}}
            invalid = Path(directory) / "invalid.toml"
            for content in ("invalid TOML", 'version = 2\n[cross]\nremap = "invalid-target"\n'):
                invalid.write_text(content)
                with self.subTest(content=content), patch.object(
                    gui.QMessageBox, "warning", return_value=gui.QMessageBox.StandardButton.Discard
                ):
                    editor._open_config(str(invalid))
                self.assertEqual(editor.config["cross"]["remap"], "circle")
                self.assertTrue(editor.document.dirty)
                self.assertEqual(editor.current_file, str(Path(directory) / "original.toml"))
            editor.document.revert()
            self.assertEqual(editor.config, original)

    def test_combo_dialog_add_remove_and_save_preserves_edits(self):
        original = [{"key": "cross", "output": "circle"}]
        dialog = ComboDialog(None, "left_paddle", original, {}, self.capabilities)
        self.addCleanup(dialog.deleteLater)
        dialog.table.cellWidget(0, 0).setCurrentText("square")
        dialog.table.cellWidget(0, 1).setCurrentText("key:space")
        dialog._add()
        self.assertEqual(dialog.table.cellWidget(0, 0).currentText(), "square")
        self.assertEqual(dialog.table.cellWidget(0, 1).currentText(), "key:space")
        dialog.table.cellWidget(1, 0).setCurrentText("triangle")
        dialog.table.cellWidget(1, 1).setCurrentText("r1")
        dialog._remove(0)
        dialog._save()
        self.assertEqual(dialog.result(), gui.QDialog.DialogCode.Accepted)
        self.assertEqual(dialog.combos, [{"key": "triangle", "output": "r1"}])
        self.assertEqual(original, [{"key": "cross", "output": "circle"}])

    def test_combo_dialog_rejects_duplicate_self_and_fn_face_keys(self):
        for modifier, keys in (
            ("left_paddle", ["cross", "cross"]),
            ("left_paddle", ["left_paddle"]),
            ("left_fn", ["cross"]),
        ):
            with self.subTest(modifier=modifier, keys=keys):
                dialog = ComboDialog(None, modifier, [
                    {"key": key, "output": "circle"} for key in keys
                ], {}, self.capabilities)
                self.addCleanup(dialog.deleteLater)
                with patch.object(gui.QMessageBox, "warning") as warning:
                    dialog._save()
                warning.assert_called_once()
                self.assertEqual(dialog.result(), gui.QDialog.DialogCode.Rejected)

    def test_real_keyboard_picker_filters_and_returns_selected_capability(self):
        picker = keyboard_dialog.KeyboardPicker(None, self.capabilities, "key:space")
        self.addCleanup(picker.deleteLater)
        role = gui.Qt.ItemDataRole.UserRole
        self.assertEqual(picker.list_widget.currentItem().data(role), "space")
        self.assertEqual(picker.list_widget.count(), len(self.capabilities.keyboard_keys))
        picker.findChild(QLineEdit).setText("ENTER")
        visible = [picker.list_widget.item(i) for i in range(picker.list_widget.count())
                   if not picker.list_widget.item(i).isHidden()]
        self.assertEqual({item.data(role) for item in visible}, {"enter", "kpenter"})
        item = next(item for item in visible if item.data(role) == "kpenter")
        picker.list_widget.setCurrentItem(item)
        picker._accept()
        self.assertEqual(picker.result(), gui.QDialog.DialogCode.Accepted)
        self.assertEqual(picker.key_name(), "key:kpenter")

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

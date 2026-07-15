from copy import deepcopy
from dataclasses import dataclass, field
import os
from pathlib import Path
import stat
import tempfile
import tomllib
from typing import Any


ConfigData = dict[str, Any]


def atomic_write_text(path: str, content: str) -> None:
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    previous_mode = stat.S_IMODE(target.stat().st_mode) if target.exists() else None
    temporary_path = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=target.parent, delete=False
        ) as temporary:
            temporary_path = temporary.name
            temporary.write(content)
            temporary.flush()
            os.fsync(temporary.fileno())
        if previous_mode is not None:
            os.chmod(temporary_path, previous_mode)
        os.replace(temporary_path, target)
        temporary_path = None
    finally:
        if temporary_path is not None:
            try:
                os.unlink(temporary_path)
            except FileNotFoundError:
                pass


def load_profile_config(path: str) -> ConfigData:
    profile_path = Path(path)
    if not profile_path.exists():
        return {"config": "default.toml", "profiles": {}}
    try:
        with profile_path.open("rb") as file:
            raw = tomllib.load(file)
    except tomllib.TOMLDecodeError as error:
        raise RuntimeError(f"cannot parse {profile_path}: {error}") from error
    except OSError as error:
        raise RuntimeError(f"cannot read {profile_path}: {error}") from error

    profiles = {}
    for name, value in raw.get("profiles", {}).items():
        profiles[name] = {
            "config": value.get("config", "default.toml"),
            "match_process": value.get("match_process", ""),
            "match_cmdline": value.get("match_cmdline", ""),
        }
    return {
        "config": raw.get("config", "default.toml"),
        "profiles": profiles,
    }


def find_macro_references(config: ConfigData, macro_name: str) -> list[str]:
    references = []
    for button, button_config in config.items():
        if button in ("macros", "version", "output_device") or not isinstance(
            button_config, dict
        ):
            continue
        if button_config.get("remap") == macro_name:
            references.append(f"{button} remap")
        for index, combo in enumerate(button_config.get("combos", []), start=1):
            if combo.get("output") == macro_name:
                key = combo.get("key") or str(index)
                references.append(f"{button} combo[{key}]")
    return references


def rename_macro(config: ConfigData, old_name: str, new_name: str) -> None:
    macros = config.setdefault("macros", {})
    if old_name not in macros:
        raise ValueError(f"Macro '{old_name}' does not exist")
    if new_name != old_name and new_name in macros:
        raise ValueError(f"Macro '{new_name}' already exists")
    if new_name == old_name:
        return

    for button_config in config.values():
        if not isinstance(button_config, dict) or button_config is macros:
            continue
        if button_config.get("remap") == old_name:
            button_config["remap"] = new_name
        for combo in button_config.get("combos", []):
            if combo.get("output") == old_name:
                combo["output"] = new_name
    macros[new_name] = macros.pop(old_name)


@dataclass
class ConfigDocument:
    data: ConfigData = field(default_factory=lambda: {"version": 2})
    current_file: str | None = None
    _saved: ConfigData = field(init=False, repr=False)

    def __post_init__(self) -> None:
        self._saved = deepcopy(self.data)

    @property
    def dirty(self) -> bool:
        return self.data != self._saved

    def mark_saved(self, current_file: str | None = None) -> None:
        if current_file is not None:
            self.current_file = current_file
        self._saved = deepcopy(self.data)

    def replace(self, data: ConfigData, current_file: str | None = None) -> None:
        self.data = data
        self.current_file = current_file
        self.mark_saved()

    def revert(self) -> None:
        self.data = deepcopy(self._saved)

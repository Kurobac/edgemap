from dataclasses import dataclass
import tomllib


CAPABILITIES_VERSION = 1


@dataclass(frozen=True)
class KeyboardKey:
    name: str
    code: int


@dataclass(frozen=True)
class Capabilities:
    output_devices: tuple[str, ...]
    source_buttons: tuple[str, ...]
    remap_targets: tuple[str, ...]
    combo_keys: tuple[str, ...]
    combo_outputs: tuple[str, ...]
    macro_step_buttons: tuple[str, ...]
    reserved_macro_names: frozenset[str]
    keyboard_keys: tuple[KeyboardKey, ...]

    @classmethod
    def from_toml(cls, content: str) -> "Capabilities":
        try:
            raw = tomllib.loads(content)
        except tomllib.TOMLDecodeError as error:
            raise ValueError(f"invalid capabilities TOML: {error}") from error
        if raw.get("version") != CAPABILITIES_VERSION:
            raise ValueError(
                f"unsupported capabilities version: {raw.get('version')!r}"
            )

        def string_tuple(name: str) -> tuple[str, ...]:
            value = raw.get(name)
            if not isinstance(value, list) or not value:
                raise ValueError(f"capabilities field '{name}' must be a non-empty array")
            if not all(isinstance(item, str) and item for item in value):
                raise ValueError(f"capabilities field '{name}' contains an invalid name")
            if len(set(value)) != len(value):
                raise ValueError(f"capabilities field '{name}' contains duplicates")
            return tuple(value)

        keyboard_raw = raw.get("keyboard_keys")
        if not isinstance(keyboard_raw, list) or not keyboard_raw:
            raise ValueError("capabilities field 'keyboard_keys' must be a non-empty array")
        keyboard_keys = []
        for item in keyboard_raw:
            if not isinstance(item, dict):
                raise ValueError("capabilities keyboard key must be a table")
            name = item.get("name")
            code = item.get("code")
            if not isinstance(name, str) or not name or not isinstance(code, int):
                raise ValueError("capabilities keyboard key has invalid name or code")
            keyboard_keys.append(KeyboardKey(name, code))
        if len({key.name for key in keyboard_keys}) != len(keyboard_keys):
            raise ValueError("capabilities keyboard key names contain duplicates")
        if len({key.code for key in keyboard_keys}) != len(keyboard_keys):
            raise ValueError("capabilities keyboard keycodes contain duplicates")

        return cls(
            output_devices=string_tuple("output_devices"),
            source_buttons=string_tuple("source_buttons"),
            remap_targets=string_tuple("remap_targets"),
            combo_keys=string_tuple("combo_keys"),
            combo_outputs=string_tuple("combo_outputs"),
            macro_step_buttons=string_tuple("macro_step_buttons"),
            reserved_macro_names=frozenset(string_tuple("reserved_macro_names")),
            keyboard_keys=tuple(keyboard_keys),
        )

    @property
    def keyboard_names(self) -> tuple[str, ...]:
        return tuple(key.name for key in self.keyboard_keys)

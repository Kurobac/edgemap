#!/usr/bin/env python3
"""Build the source GUI package into the committed single-file executable."""

import os
from pathlib import Path
import sys
import zipfile


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "gui"
DEFAULT_OUTPUT = ROOT / "edgemap-gui-v6.py"
ZIP_TIMESTAMP = (1980, 1, 1, 0, 0, 0)


def build(output: Path) -> None:
    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(f".{output.name}.tmp")
    try:
        temporary.write_bytes(b"#!/usr/bin/env python3\n")
        with zipfile.ZipFile(
            temporary, mode="a", compression=zipfile.ZIP_DEFLATED, compresslevel=9
        ) as archive:
            for source in sorted(SOURCE.rglob("*.py")):
                if "__pycache__" in source.parts:
                    continue
                relative = source.relative_to(SOURCE).as_posix()
                info = zipfile.ZipInfo(relative, ZIP_TIMESTAMP)
                info.create_system = 3
                info.external_attr = 0o100644 << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, source.read_bytes(), compresslevel=9)
        os.chmod(temporary, 0o755)
        os.replace(temporary, output)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def main(argv: list[str]) -> int:
    if len(argv) > 2:
        print(f"Usage: {argv[0]} [OUTPUT]", file=sys.stderr)
        return 1
    build(Path(argv[1]) if len(argv) == 2 else DEFAULT_OUTPUT)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))

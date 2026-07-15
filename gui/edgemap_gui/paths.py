import os
from collections.abc import Mapping


def edgemap_config_dir(env: Mapping[str, str] | None = None) -> str:
    env = os.environ if env is None else env
    xdg = env.get("XDG_CONFIG_HOME", "")
    if xdg and os.path.isabs(xdg):
        return os.path.join(xdg, "edgemap")
    home = env.get("HOME", "")
    if not home:
        raise RuntimeError("HOME is not set or is empty")
    return os.path.join(home, ".config", "edgemap")

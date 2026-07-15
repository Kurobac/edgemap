from dataclasses import dataclass
import os
import subprocess
import tempfile

from .capabilities import Capabilities


class EdgemapClientError(RuntimeError):
    pass


@dataclass(frozen=True)
class EdgemapClient:
    binary: str = "edgemap"
    timeout_seconds: int = 10

    @classmethod
    def from_environment(cls) -> "EdgemapClient":
        return cls(os.environ.get("EDGEMAP_BINARY", "edgemap"))

    def _run(self, *args: str) -> subprocess.CompletedProcess[str]:
        try:
            return subprocess.run(
                [self.binary, *args],
                capture_output=True,
                text=True,
                timeout=self.timeout_seconds,
            )
        except FileNotFoundError as error:
            raise EdgemapClientError("edgemap binary not found in PATH") from error
        except subprocess.TimeoutExpired as error:
            raise EdgemapClientError("edgemap command timed out") from error
        except OSError as error:
            raise EdgemapClientError(f"failed to run edgemap: {error}") from error

    def capabilities(self) -> Capabilities:
        result = self._run("capabilities")
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip()
            raise EdgemapClientError(f"capabilities query failed: {detail}")
        try:
            return Capabilities.from_toml(result.stdout)
        except ValueError as error:
            raise EdgemapClientError(str(error)) from error

    def validate_path(self, path: str) -> None:
        result = self._run("validate", path)
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip()
            raise EdgemapClientError(f"config validation failed: {detail}")

    def validate_content(self, content: str) -> None:
        try:
            with tempfile.NamedTemporaryFile(mode="w", suffix=".toml") as file:
                file.write(content)
                file.flush()
                self.validate_path(file.name)
        except OSError as error:
            raise EdgemapClientError(f"cannot prepare config validation: {error}") from error

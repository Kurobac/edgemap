#!/usr/bin/env python3
"""Play a quad 48 kHz test through the Bluetooth haptics PipeWire sink."""

import math
import struct
import subprocess
import tempfile
import wave


def main():
    rate = 48000
    with tempfile.NamedTemporaryFile(suffix=".wav") as audio:
        with wave.open(audio.name, "wb") as wav:
            wav.setparams((4, 2, rate, 0, "NONE", "not compressed"))
            # Front pair (must not vibrate), left, pause, right, stop.
            for channels, duration in [((0, 1), 0.5), ((2,), 1), ((), 0.5), ((3,), 1), ((), 0.2)]:
                count = round(rate * duration)
                data = bytearray()
                for i in range(count):
                    ramp = min(i, count - 1 - i, rate // 100) / (rate // 100)
                    sample = round(math.sin(math.tau * 75 * i / rate) * 6000 * ramp)
                    data.extend(struct.pack("<4h", *(sample if ch in channels else 0 for ch in range(4))))
                wav.writeframes(data)
        subprocess.run(["pw-play", "--target", "edgemap.dualsense", audio.name], check=True)


if __name__ == "__main__":
    main()

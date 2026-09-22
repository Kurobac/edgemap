#!/usr/bin/env python3
"""Play speaker and HD haptics through the same quad PipeWire sink."""

import math
import struct
import subprocess
import tempfile
import wave


def main():
    rate = 48000
    # FL/FR speaker, RL/RR actuators. Separate tones reveal routing mistakes;
    # the final segment exercises both BT packet types at the same time.
    stages = [
        ((440, 440, 0, 0), 1.0),
        ((0, 0, 0, 0), 0.4),
        ((0, 0, 75, 0), 1.0),
        ((0, 0, 0, 0), 0.4),
        ((0, 0, 0, 75), 1.0),
        ((0, 0, 0, 0), 0.4),
        ((660, 660, 75, 75), 2.0),
        ((0, 0, 0, 0), 0.4),
    ]
    with tempfile.NamedTemporaryFile(suffix=".wav") as audio:
        with wave.open(audio.name, "wb") as wav:
            wav.setparams((4, 2, rate, 0, "NONE", "not compressed"))
            for frequencies, duration in stages:
                count = round(rate * duration)
                data = bytearray()
                for i in range(count):
                    ramp = min(i, count - 1 - i, rate // 100) / (rate // 100)
                    values = [round(math.sin(math.tau * hz * i / rate) * 6000 * ramp)
                              for hz in frequencies]
                    data.extend(struct.pack("<4h", *values))
                wav.writeframes(data)
        print("顺序：扬声器 → 左震动 → 右震动 → 扬声器和双侧震动同时播放 → 停止", flush=True)
        subprocess.run(["pw-play", "--target", "edgemap.dualsense", audio.name], check=True)


if __name__ == "__main__":
    main()

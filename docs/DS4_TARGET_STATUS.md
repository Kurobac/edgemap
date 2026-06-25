# DS4 Target — Beta Status

## Current conclusion

`output_device = "dualshock4"` is no longer considered architecturally blocked.
The old conclusion that UHID DS4 is fundamentally unusable under Proton was
based on incomplete device-identity evidence and is obsolete.

The DS4 HID/report side in dseuhid is good enough for Beta use: buttons, sticks,
triggers, touchpad coordinates, LED output, rumble conversion, SDL/evdev, and
Steam controller tests all work. Several native DS4 games can use the UHID DS4
target with Steam Input disabled and hidraw enabled.

The remaining Proton compatibility issue is Windows device identity. Some games
expect a real USB DS4 HID interface path like:

```text
USB\VID_054C&PID_09CC&MI_03\...
```

For dseuhid's UHID DS4, unpatched Proton previously exposed the hidraw device
without `MI_03`, with version `0000` and `input = -1`. Games such as CP2077 then
stopped after the first `GetFeature(0x12)` and never entered the full Sony
feature init path.

With the Proton DS4 UHID MI_03 identity patch, the hidraw device is exposed as
`VID_054C&PID_09CC&MI_03`, version `0100`, `input = 3`. CP2077 then proceeds
through the same full init sequence as a physical DS4.

## Verified behavior

| Area | Status |
|------|--------|
| DS4 input report format | Working |
| Buttons / sticks / triggers | Working |
| Touchpad click + coordinates | Working |
| LED output | Working |
| Rumble | Working via DS4 output to DS5 output conversion |
| SDL/evdev path | Working |
| Native DS4 hidraw games | Beta; varies by Proton identity path |
| Proton with DS4 MI_03 patch | CP2077 recognizes input and performs full DS4 feature init |

## Real DS4 vs UHID DS4 logs

Physical DS4 in CP2077:

```text
read feature report id 18  length 64
write output report id 5   length 32
read feature report id 18  length 16
read feature report id 163 length 49
write feature report id 20 length 17
read feature report id 2   length 37
```

Unpatched UHID DS4 in CP2077 and SOTTR:

```text
read feature report id 18  length 64
write output report id 5   length 32
```

This partial sequence does **not** mean the DS4 target necessarily fails. SOTTR
still accepts input and shows Sony-style prompts with only the short init path.
It means the game did not enter the deeper Sony feature path, so gyro, firmware,
or calibration behavior may differ by title.

Patched Proton UHID DS4 in CP2077:

```text
USB\VID_054C&PID_09CC&MI_03\...
read feature report id 18  length 64
write output report id 5   length 32
read feature report id 18  length 16
read feature report id 163 length 49
write feature report id 20 length 17
read feature report id 2   length 37
```

## Proton patch

The patch lives in the
[proton-eg-patch repository](https://github.com/Kurobac/proton-eg-patch/blob/main/patches/proton-ds4-uhid-mi03.patch).

dseuhid does not ship or install Proton. The README points users to the patch
repository because Proton-side identity is outside dseuhid's HID report path.

Patch behavior:

- Applies only to UHID-backed DS4 hidraw devices.
- Matches Sony DS4 PIDs `054C:05C4` and `054C:09CC`.
- Leaves evdev input subnodes alone.
- Sets `input = 3`, `version = 0x0100` when missing, and a stable container id.

## Implemented in dseuhid

| Feature | Status | File |
|---------|--------|------|
| DS4 HID descriptor | Implemented | `src/descriptor.rs` |
| DS4 input report writer | Implemented | `src/report.rs` |
| DS4 output report conversion | Implemented | `src/report.rs` |
| Feature report cache (`0x02`, `0x12`, `0xA3`) | Implemented | `src/main.rs` |
| DS4 virtual identity (`Wireless Controller`, PID `0x09CC`) | Implemented | `src/main.rs` |
| `output_device = "dualshock4"` config | Beta | `src/config.rs` |
| GUI Output Device selector | Beta | `edgemap-gui-v6.py` |
| UHID recreation on output device change | Implemented | `src/proxy.rs` |

## Compatibility notes

- DS4 target is Beta, not a promise that every native DS4 game works.
- Games that only need input and Sony prompts may work without the Proton patch.
- Games that require the complete Sony feature init are expected to need the
  Proton DS4 UHID MI_03 identity patch.
- The DS4 target does not emulate an entire USB device tree in Linux; the
  compatibility fix belongs on the Proton device-enumeration side.

# DS4 Target — Final Status Report

## ✅ Verified Working

| Component | Verification |
|-----------|-------------|
| Raw input report format | Byte-for-byte match with Windows reWASD capture |
| Buttons / sticks / triggers | Steam + evtest + Genshin Impact hidraw path |
| Touchpad (click + coordinates) | evtest + Steam controller test (Y-axis 1080→942 scaling) |
| LED output | Verified in Steam and Genshin Impact |
| Rumble | Steam vibration test passed (DS4→DS5 compatible vibration) |
| SDL/evdev path | sdl2-jstest works |
| Kernel driver probe | UHID start/open succeeds, device nodes created |
| DS Edge passthrough mode | Works |
| DS5 forced mode | Works |

## ❌ Game Compatibility — Final Conclusion

### Category A — HIDRAW-path games completely ignore the controller (CP2077 / TLOU / Death Stranding)

**Symptom:** All native DS4 games ignore the controller entirely — no button response.
Genshin Impact recognizes the controller but gyro and vibration don't work.

**Root cause:** The game stops after `GetFeature(0x12)` — it never queries `GetFeature(0xA3)`, never sends `SetFeature(0x14)`. **Even with all HID data and Proton layers aligned, the game refuses to proceed with the full init sequence.**

### Complete debug matrix

| Attempt | Result |
|---------|--------|
| HID raw report aligned to reWASD | ✅ Data correct, game still unresponsive |
| Feature report data fix | ✅ Garbage-data bug fixed |
| HID Descriptor Report Count fix | ✅ Caps parsed correctly |
| Device name = "Wireless Controller" | ✅ Matching real DS4 |
| MAC / uniq / fw_version non-zero | ✅ Matching real DS4 |
| PID 0x05C4 / 0x09CC | ✅ Both show same behavior |
| **DS4→DS4 pure passthrough demo** (real DS4 data via UHID) | ❌ Still broken → **Not a data generation problem** |
| **Proton ContainerID patch** (bus_udev.c) | ❌ GUID_NULL fixed, no effect |
| **Proton SetupAPI recursive enum patch** (devinst.c) | ✅ Device count 4→21, no effect |

**Conclusion:** UHID virtual DS4 faces a fundamental architectural barrier under Proton. The game's refusal to initialize is NOT caused by HID data or descriptor issues. dseuhid has done everything possible from its side.

## 🔍 Investigation Timeline

### Windows API Monitor — CP2077 init sequence (working)

| # | Thread | API | Report | Size | Result |
|---|--------|-----|--------|------|--------|
| 1 | 73 | HidD_GetFeature | 0x12 (MAC) | 16B | TRUE |
| 2 | 77 | HidD_GetFeature | 0xA3 (firmware) | 49B | TRUE |
| 3 | 77 | **HidD_SetFeature** | **0x14** | **17B** | **TRUE** |
| 4 | 77 | HidD_GetFeature | 0x02 (calibration) | 37B | TRUE |

### Real DS4 (0x09CC) Proton log — full init sequence

```
read feature report id 18  length 64:   ← winedevic enumeration of 0x12
write output report id 5   length 32:   ← LED init
read feature report id 18  length 16:   ← Game's 2nd query of 0x12 (correct size!)
read feature report id 163 length 49:   ← 0xA3 firmware
write feature report id 20 length 17:   ← SetFeature 0x14 init!
read feature report id 2   length 37:   ← 0x02 calibration
```

### Our DS4 Proton log — stuck at step 1

```
read feature report id 18  length 64:   ← Just this one
write output report id 5   length 32:   ← LED
[STOP — no further feature report operations]
```

### Linux strace: real DS4 vs our DS4

| ioctl | Real DS4 | DS4 UHID |
|-------|----------|----------|
| HIDIOCGFEATURE(64) winedevic | ✅ | ✅ |
| HIDIOCGFEATURE(64) game | ✅ | ✅ |
| **HIDIOCGFEATURE(16)** game | ✅ | ❌ |
| HIDIOCGFEATURE(49) | ✅ | ❌ |
| **HIDIOCSFEATURE(17)** | ✅ | ❌ |
| HIDIOCGFEATURE(37) | ✅ | ❌ |

### DS4→DS4 Pure Passthrough Demo

A standalone binary (`src/bin/ds4-passthru.rs`) was written: real DS4 physical device → UHID virtual DS4 pure passthrough. **Real data + UHID → game still doesn't work.** Confirms the issue is not in data generation.

### Proton source patches attempted

| Patch | File | Effect |
|-------|------|--------|
| DS5 ContainerID extension | `winebus.sys/main.c` | ❌ UHID doesn't use this path |
| ContainerID fallback | `winebus.sys/bus_udev.c` | ✅ GUID_NULL fixed, but no help |
| **SetupAPI ROOT recursive enum** | `setupapi/devinst.c` | ✅ Device count 4→21, still no effect |

All three patches under `/home/kurobac/Projects/proton-eg-patch/patches/`.

## 🐛 Bugs Found and Fixed

### 1. Feature Report data filled with report ID byte (🔴 P0)
```rust
// ❌ vec![0x12u8; 16] fills all 16 bytes with 0x12; resize(16, 0) is a no-op
// ✅ vec![0u8; 16]; buf[0] = 0x12;
```
Affected three reports: 0x02, 0x12, 0xA3. All bytes not explicitly assigned were garbage.

### 2. HID Descriptor Report Count off-by-one (🔴 P0)
| Report | Before (wrong) | After (correct) |
|--------|---------------|-----------------|
| 0x02 cal | 0x25(37) | 0x24(36) |
| 0x12 MAC | 0x10(16) | 0x0F(15) |
| 0xA3 fw | 0x31(49) | 0x30(48) |
HID spec: Report Count excludes the report ID byte. PSDevWiki totals include it.
After fix: MAC no longer re-queried infinitely by Proton (20+ → 1).

### 3. Device name
```rust
let name = if output_device == "dualshock4" {
    "Wireless Controller"  // Matches ViGEmBus / real DS4
};
```

### 4. SET_REPORT forwarding
DS4 mode: kernel-initiated SET_REPORT (rnum=0x13) not forwarded to physical device; reply success directly.

### 5. Other fixes
- `uniq` → `"c0:13:37:00:00:01"`
- firmware version → `0x0049` (real DS4 value)
- sensor temperature → `0x06`
- battery status → `0x1B` (Full + USB connected)
- 0x12 report bytes 7-9 → copied from real DS4
- 0xA3 report → full build date string + hw_version/fw_version

### Proton `debug_print_preparsed` confirms correctness

```
feature 5: RId 18 (0x12), RCnt 15 ✅
feature 7: RId 20 (0x14), RCnt 16 ✅
-- All 43 feature value caps parsed correctly
```

## 💡 Final Conclusion

**DS4 Target's UHID simulation is completely correct at the HID level** — verified with Steam, evtest, and SDL. The game incompatibility is an architectural limitation of UHID virtual devices under Proton: the game abandons initialization after `GetFeature(0x12)`, and this behavior is unaffected by HID data, descriptors, ContainerID, or SetupAPI enumeration.

DS4 Target HID code is worth preserving. Once someone breaks through from the Proton side, it can be enabled immediately.

## Changes Worth Merging to Main

| Change | Files | Notes |
|--------|-------|-------|
| `force_dualsense` → `output_device` enum | `config.rs`, `main.rs`, `proxy.rs`, `edgemap-gui-v6.py` | Three modes (auto/dualsense/dualshock4) |
| Feature Report data bug fix | `main.rs` | `vec![0u8; SIZE]` + `buf[0] = report_id` |
| HID Descriptor Report Count fix | `descriptor.rs` | Three reports RCnt -1 |
| Non-zero feature report data | `main.rs` | MAC, fw_version, calibration, battery, sensor temp |
| GUI Output Device dropdown | `edgemap-gui-v6.py` | Replaces force_dualsense checkbox |

## 📊 Implemented Feature Checklist

| Feature | Status | File |
|---------|--------|------|
| DS4 HID descriptor (467 bytes) | ✅ | `src/descriptor.rs` |
| Input report writer (DS4 64-byte layout) | ✅ | `src/report.rs` |
| Output report converter (DS4→DS5 rumble+LED) | ✅ | `src/report.rs` |
| Touchpad coordinate forwarding (Y 1080→942) | ✅ | `src/report.rs` |
| Gyro/accelerometer forwarding (1:1 calibration) | ✅ | `src/report.rs` + `src/main.rs` |
| Feature report cache (0x02/0x12/0xA3) | ✅ | `src/main.rs` |
| Output device enum (auto/dualsense/dualshock4) | ✅ | `src/config.rs` |
| GUI Output Device dropdown | ✅ | `edgemap-gui-v6.py` |
| UHID reload detection for output_device changes | ✅ | `src/proxy.rs` |
| Sensor temp + battery status + touch INACTIVE | ✅ | `src/report.rs` |
| DS4 pure passthrough verification demo | ✅ | `src/bin/ds4-passthru.rs` |

## 📝 Artifacts

- Proton source patches (`/home/kurobac/Projects/proton-eg-patch/patches/`)
  - `proton-ds4-uhid-containerid.patch` — bus_udev.c UHID ContainerID fallback
  - `proton-ds4-setupapi-root-enum.patch` — setupapi/devinst.c ROOT recursive enumeration
- Windows API Monitor capture (CP2077 init sequence)
- reWASD raw HID report byte-for-byte comparison data
- Real DS4 (0x09CC) hardware comparison testing

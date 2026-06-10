# HD Haptics Fix — Proton DualSense ContainerId Patch

## Problem

With dseuhid running, Cyberpunk 2077 loses DualSense HD haptics (audio-channel vibration).
Genshin Impact works because it enumerates all audio endpoints without requiring HID-audio device correlation.

## Root Cause

DualSense HD haptics use the USB audio interface (`snd-usb-audio`), not HID.
Windows games correlate the HID device with the audio device via **ContainerId** (a GUID):

```
ContainerId = { Data1(VID+PID) - Data2(USB bus) - Data3(USB dev) - Data4(USEC_INITIALIZED) }
```

| Device | Source | ContainerId |
|--------|--------|-------------|
| Physical hidraw (no dseuhid) | sysfs → USB parent → BUSNUM/DEVNUM/USEC | `{0DF2054C-0005-0006-XXXX-XXXXXXXX}` |
| Audio endpoint | sysfs → USB parent → BUSNUM/DEVNUM/USEC | `{0DF2054C-0005-0006-XXXX-XXXXXXXX}` — matches |
| **UHID virtual device** | No USB parent → fallback random | `{0DF2054C-0000-FFFF-XXXX-XXXXXXXX}` — **mismatch** |

The UHID device lives under `/sys/devices/virtual/` and has no USB parent device.
Proton cannot obtain BUSNUM/DEVNUM/USEC_INITIALIZED from the sysfs tree,
so the generated ContainerId differs from the audio endpoint's.
The game cannot associate the two devices → no HD haptics data is sent.

### Key Code Paths

1. **HID side** — `winebus.sys/main.c:297` `get_container_id()`
   - Prefers `desc.bus_container_id` (computed from USB topology; only set for physical hidraw)
   - UHID has no USB parent → falls back to `make_unique_container_id()` random value

2. **Audio side (PulseAudio)** — `winepulse.drv/pulse.c:592` `get_container_id()`
   - Walks up sysfs tree to find USB parent device
   - Computes ContainerId from real USB topology data

3. **Audio side (ALSA)** — `winealsa.drv/alsa.c:2412`
   - ContainerId query returns `E_NOTIMPL` (not implemented)

## Fix

Special-case DualSense (054C:0CE6) and DualSense Edge (054C:0DF2):
both the HID side and audio side compute a fixed ContainerId using Sony VID only,
with a marker to identify this patch in debug logs.

```
// All three sides return: {0000054C-ED6E-0001-0000-000000000000}
// VID=0x054C, Data2=0xED6E ("edgemap"), Data3=0x0001, Data4 all zero.
// Using VID only (not VID+PID) ensures force_dualsense works — the virtual
// device may report a different PID but the real audio interface VID is unchanged.
```

### Files Modified (3)

| File | Function / Location | Change |
|------|--------------------|--------|
| `wine/dlls/winebus.sys/main.c` | `get_container_id()` | When `bus_container_id` is NULL and device is Sony pad → hardcoded GUID |
| `wine/dlls/winepulse.drv/pulse.c` | `get_container_id()` | After USB topology computation, override Sony pads with same |
| `wine/dlls/winealsa.drv/alsa.c` | `alsa_get_prop_value()` | Add `DEVPKEY_Device_ContainerId` property handler |

### Side Effects

When a physical DualSense is connected directly (no dseuhid), its ContainerId changes from
USB-topology-based to a fixed value. ContainerId's sole purpose is cross-device correlation —
as long as HID and Audio remain consistent, there is no functional impact.
Multiple DualSense pads would share the same ContainerId (dseuhid only virtualizes one).

## Build

```bash
# 1. Obtain the wine source matching your GE-Proton version.
#    Check GE-Proton's wine submodule commit:
#    git -C proton-ge-custom submodule status wine

# 2. Apply patch
cd <wine-src>
patch -p1 < docs/proton-dualsense-containerid.patch

# 3. Build
./autogen.sh
mkdir build64 && cd build64
../configure --enable-win64
make -j$(nproc)

# 4. Replace in GE-Proton installation
DEST=~/.local/share/Steam/compatibilitytools.d/GE-ProtonXX-XX/files/lib/wine

# PE side (winebus.sys main.c changes)
cp build64/dlls/winebus.sys/x86_64-windows/winebus.sys \
   $DEST/x86_64-windows/winebus.sys

# Unix side (winepulse, winealsa changes)
strip build64/dlls/winepulse.drv/winepulse.so
cp build64/dlls/winepulse.drv/winepulse.so $DEST/x86_64-unix/winepulse.so
strip build64/dlls/winealsa.drv/winealsa.so
cp build64/dlls/winealsa.drv/winealsa.so   $DEST/x86_64-unix/winealsa.so
cp build64/dlls/winebus.sys/winebus.so     $DEST/x86_64-unix/winebus.so

# Note: winebus.sys changes are on the PE side (winebus.sys),
#        winepulse/alsa changes are on the Unix side (.so).
#        See dlls/*/Makefile.in SOURCES for the split.
```

## Verification

```bash
# Steam CP2077 launch options
WINEDEBUG=+hid PROTON_LOG=1 %command%

# Check ContainerId
grep "0000054C" ~/steam-1091500.log

# Expected:
# {0000054C-ED6E-0001-0000-000000000000}
```

## Related Files

- `docs/proton-dualsense-containerid.patch` — patch file for ValveSoftware/wine
- This issue cannot be fixed on the dseuhid side (kernel UHID interface does not support setting sysfs USB parent)

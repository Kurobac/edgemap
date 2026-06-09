# DS4 debug tools

## hid-inspect.exe (Windows / Wine)

Enumerates Sony HID devices and runs detailed HID API inspection matching game behavior.

```bash
# Build (Linux, mingw-w64)
x86_64-w64-mingw32-gcc -s -o hid-inspect.exe hid-inspect.c -lsetupapi -lhid

# Run on Linux with Wine
wine hid-inspect.exe              # all Sony devices
wine hid-inspect.exe 05c4         # DS4 only
wine hid-inspect.exe 0ce6         # DS5 only

# Copy to Windows and run
hid-inspect.exe 05c4
```

Output includes:
- `HidP_GetCaps` → full caps summary
- `HidP_GetLinkCollectionNodes`
- `HidP_GetSpecificButtonCaps(OUTPUT, page=0x0F, usage=0x9A)` → Sony SDK query
- `HidP_GetSpecificButtonCaps(INPUT, page=Button, usage=0)`
- `HidP_GetSpecificValueCaps(INPUT, page=1, usage=X/Y)` → stick axes
- `HidP_GetSpecificValueCaps(INPUT, page=0x02, usage=0xC8)` → touch related
- Feature reports 0x12/0xA3/0x02/0x14

## ds4-probe (Linux hidraw)

Direct hidraw ioctl test — bypasses Wine entirely.

```bash
# Build
gcc -s -o ds4-probe ds4-probe.c

# Find device node
for d in /sys/class/hidraw/hidraw*; do
  grep -q '054C.*05C4' "$d/device/uevent" 2>/dev/null && echo "$d" | sed 's|.*/||'
done

# Run (needs root or hidraw access)
sudo ./ds4-probe /dev/hidrawN
```

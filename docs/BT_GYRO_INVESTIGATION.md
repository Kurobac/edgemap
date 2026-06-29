# Bluetooth Source Gyro Investigation

Date: 2026-06-29

This note records the current investigation around DualSense Bluetooth source
input being exposed as a virtual USB DualSense target, especially the Genshin
Impact gyro behavior. It is intentionally a test log, not a final design.

## Scope

- Physical source: Sony DualSense / DualSense Edge over Bluetooth hidraw.
- Virtual target: USB DualSense UHID target.
- Main symptom: gyro axes broadly point in the correct direction, but Genshin
  Impact shows periodic aim jumps or drift when using the virtual USB target.
- USB physical source with the same virtual USB target path does not show this
  gyro issue.

## Observed Game Behavior

- In Genshin Impact, direct Bluetooth DualSense does not appear to support
  DualSense gyro.
- Direct Bluetooth DualSense shows DS4-style colored face-button icons.
- Direct USB DualSense shows DualSense-style monochrome face-button icons.
- With dseuhid BT source -> USB target, gyro direction is generally correct,
  but aim can periodically jump.
- The drift direction depends on the controller posture when aim mode starts.
  This suggests the raw motion axes are probably not simply swapped or inverted.

## Captured Bluetooth Trace

Capture file used during the investigation:

```text
genshin-bt.snoop
```

The capture was started before launching the game. The trace showed:

- Many HIDP input reports with payload prefix `a1 31`.
- Two HIDP output reports with payload prefix `a2 31`.
- No observed Bluetooth feature report matching the virtual USB target's
  Genshin `0x08` SET_REPORT payload.

This suggests Genshin's direct Bluetooth path is different from the virtual USB
target path, and it is not a reliable source for the USB target's `0x08`
feature-report behavior.

## SET_REPORT 0x08 Test

Genshin sends this SET_REPORT to the virtual USB DualSense target at startup:

```text
rnum=0x08, size=48, report_id=0x08
data=08 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
```

Tests performed:

- Naively forwarding the USB-shaped `0x08` report to the Bluetooth physical
  controller with HIDIOCSFEATURE failed with `EIO`.
- Appending a PlayStation feature-report CRC tail also failed with `EIO`.
- Temporarily dropping USB physical `0x08` in the USB-source path did not break
  USB gyro in Genshin.

Current conclusion:

- Genshin's `0x08` report is not proven to be the cause of the BT-source gyro
  issue.
- BT SET_REPORT forwarding should remain disabled until the real Bluetooth
  feature-report transport rule is known.

## Motion Timestamp Tests

The DS5 common input payload contains motion data and a sensor timestamp. The
Bluetooth report carries a common payload that can be converted to a USB-shaped
64-byte report, but the timestamp domain differs from real USB behavior.

Observed real USB behavior:

- USB `raw[7]` sequence increments `0..255`.
- USB `raw[28..32]` sensor timestamp usually increases by about `3000` per
  frame.
- Some real USB deltas are around `2816/2817` with occasional `3755/3756`.

Observed Bluetooth-backed payload behavior:

- BT common payload `raw[7]` is fixed at `0x01`.
- BT sensor timestamp has a different pattern and can contain large jumps.

Tests performed:

- Forwarding BT timestamp as-is: Genshin gyro had strong periodic drift/jumps.
- Synthesizing fixed `+3000` per emitted USB input frame: seemed somewhat
  better in manual testing, but did not fully fix the issue.
- Synthesizing a five-frame rhythm of `+2817, +2817, +2817, +2817, +3756`: no
  clear improvement over fixed `+3000`.
- Synthesizing HHD-style `monotonic_ns / 333`: no clear improvement over fixed
  `+3000`.
- Reverting USB target `raw[7]` to proxy-generated sequence instead of carrying
  BT `0x01`: kept, because a virtual USB target should not expose the BT fixed
  sequence byte.

Later manual tests weakened the fixed-`+3000` conclusion. The temporary
timestamp/motion/mask debug switches were useful for isolating the issue, but
the final retained workaround is the fixed-rate repeat path described below.

## Latest Findings

Additional tests after adding motion replay/masking switches:

- `DSEUHID_BT_USB_MASK=all` and individual masks for
  `reserved`/`host_ts`/`device_ts`/`temp`/`status` did not produce a stable,
  obvious behavioral difference.
- Replaying real USB `raw[16..28]` gyro/accel axes into the BT-source target
  path still produced visible gyro jitter.
- `DSEUHID_MOTION_REPLAY_MODE=zero_gyro` greatly reduced the jitter amplitude,
  but did not remove all movement.
- `DSEUHID_MOTION_REPLAY_MODE=neutral_accel` did not show a clear difference.
- `fixed_ts` and the default BT-backed timestamp were hard to distinguish in
  coarse manual testing, but careful comparison showed:
  - default BT-backed timestamp: jitter tends to alternate left/right, with
    little net displacement;
  - `fixed_ts` (`+3000` per emitted frame): lighter but more one-directional
    left drift;
  - `zero_ts`: no visible jitter, but horizontal gyro aiming stops responding.

Current interpretation:

- Gyro values contribute to the visible amplitude, but `zero_gyro` still having
  slight movement means raw gyro noise is not the whole problem.
- `zero_ts` freezing horizontal gyro response strongly suggests Genshin uses the
  DS5 sensor timestamp for at least part of its motion integration/filtering.
- The largest remaining hypothesis is a cadence/timestamp mismatch. The original
  passthrough path emitted one UHID input immediately for each physical BT
  report, so the game saw the hidraw/epoll arrival jitter directly. A USB
  DualSense target appears to need a steadier virtual input cadence.
- Fixed-rate repeat tests support this cadence hypothesis:
  - `DSEUHID_BT_DS5_USB_REPEAT_HZ=1000 DSEUHID_BT_DS5_USB_REPEAT_MODE=seq_ts`
    made the left/right
    jitter much worse, likely because repeated frames advanced the sensor
    timestamp while carrying the same physical gyro sample.
  - `DSEUHID_BT_DS5_USB_REPEAT_HZ=1000 DSEUHID_BT_DS5_USB_REPEAT_MODE=seq_only`
    almost completely
    removed the visible jitter while preserving usable motion.
  - `DSEUHID_BT_DS5_USB_REPEAT_MODE=passthrough` reproduces the original
    one-physical-frame-to-one-UHID-frame behavior and still drifts badly.
  - `250Hz` `seq_only` was retested and does not show the original severe drift,
    even though BT input itself is also roughly in this rate class. This points
    at cadence stability rather than raw average report rate as the important
    part.
  - `500Hz` and `1000Hz` `seq_only` were hard to distinguish in manual testing.
  - This suggests repeat frames should advance the USB report sequence byte but
    should not claim a new IMU sample by advancing `raw[28..32]`.

## External Implementation Notes

### InputPlumber

InputPlumber builds DualSense UHID reports from an internal target state rather
than passing through a physical source report.

Relevant observations:

- It writes gyro/accel fields directly into the DS5 common payload.
- It increments the report sequence byte.
- It increments an internal `sensor_timestamp` by `3` on input events.

This is useful as evidence that synthetic DS5 report generation is viable, but
the timestamp unit does not directly match the raw USB captures from this
project.

### HHD

HHD's DualSense virtual target also builds reports from scratch.

Relevant observations:

- It defines `DS5_EDGE_DELTA_TIME_NS = 333`.
- For IMU timestamp events, it writes `event_timestamp_ns / 333` into the DS5
  sensor timestamp field.
- For fake/failover timestamps, it writes `time.perf_counter_ns() / 333`.
- It can sync report emission to IMU timestamp events.

Tests performed:

- HHD's USB DualSense target was tested with Genshin on a handheld, but mapping
  to USB DualSense made the game extremely slow, around one frame per ten
  seconds.

Current conclusion:

- HHD is useful as reference material, but it is not currently a reliable oracle
  for Genshin USB DualSense behavior in this environment.

## Current Conclusion

The raw gyro/accel axes are probably mostly correct, but Genshin appears
sensitive to timestamp progression and report cadence.

Current best answer from manual testing:

- For BT source -> DS5 USB target, normalize the UHID input cadence instead of
  forwarding each physical BT report at its original arrival time.
- Repeat frames should advance `raw[7]` sequence.
- Repeat frames should keep `raw[28..32]` sensor timestamp unchanged.
- Only real physical BT frames should bring a new sensor timestamp.
- `1000Hz` is the current default because real USB DualSense input is close to
  1000Hz. The `250Hz` retest suggests the fix is not simply "more frames"; a
  steady cadence is the key behavior.

## Retained Behavior and Controls

BT source -> DS5 USB target uses fixed-rate repeat by default.

- Applies only to BT source -> DS5 USB target.
- Default is `1000Hz` with `seq_only` behavior.
- `DSEUHID_BT_DS5_USB_REPEAT_HZ` overrides the repeat rate and accepts
  `1..=2000`.
- `DSEUHID_BT_DS5_USB_REPEAT_MODE=passthrough` disables repeat and restores the
  original behavior: one UHID input per physical BT input.
- After the first valid physical BT frame, repeats the latest target report at
  the requested rate.
- Repeated frames advance `raw[7]` sequence.
- The default behavior is `DSEUHID_BT_DS5_USB_REPEAT_MODE=seq_only`, which
  keeps `raw[28..32]` unchanged on repeated frames.
- `DSEUHID_BT_DS5_USB_REPEAT_MODE=seq_ts` is retained only as a comparison mode;
  it made jitter much worse in Genshin.

## Removed Debug Controls

The following temporary switches were removed from code after the investigation:

- `DSEUHID_BT_USB_MASK`
- `DSEUHID_MOTION_CAPTURE`
- `DSEUHID_MOTION_REPLAY`
- `DSEUHID_MOTION_REPLAY_MODE`

Their findings remain documented above, but the implementation now keeps only
the repeat path.

## Remaining Follow-Up

- Validate the default `1000Hz seq_only` behavior outside Genshin.
- If another game is sensitive to repeat rate, compare `250Hz`, `500Hz`, and
  `1000Hz` before changing the default.
- Remove or hide `seq_ts` later unless further debugging needs it.
- Optionally add lightweight cadence statistics if another game shows similar
  behavior.

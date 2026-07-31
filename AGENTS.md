# AGENTS.md — edgemap

DualSense userspace UHID proxy with two binaries: `dseuhid` (proxy daemon) and `edgemap` (user CLI/daemon). The data path is physical hidraw → source codec → controller frame → L1/L2/L3 pipeline → target codec → USB UHID. Physical output travels back through the target and physical codecs. Runtime ownership uses one epoll loop without an async runtime.

## Normal verification

```bash
cargo build
cargo test
cargo run -- version
cargo run -- help
cargo run --bin edgemap -- help
PYTHONPATH=gui python3 -m edgemap_gui
QT_QPA_PLATFORM=offscreen python3 -m unittest discover -s tests -p 'test_gui.py' -v
```

Use `docs/INSTALLATION_TESTING.md` only for installation, packaging, release, or system-level verification tasks.

## Main code areas

- `src/main.rs`, `src/daemon.rs`, `src/session.rs`: process lifecycle and controller sessions.
- `src/proxy/`: epoll ownership, L1→L2→L3 transforms, timing, repeat cadence, and UHID events.
- `src/codec/`, `src/descriptor.rs`: physical/virtual wire formats, feature handling, identities, and descriptors.
- `src/config/`, `src/mapping.rs`, `src/model.rs`, `src/keycodes.rs`: schema, validation, compilation, remapping, and transport-neutral state.
- `src/control/`: versioned complete-packet Unix control protocol and daemon locks.
- `src/device/`, `src/uhid.rs`, `src/keyboard.rs`: device discovery, hidraw, UHID, and keyboard lifecycles.
- `src/bin/edgemap/`: CLI, profile monitoring, notifications, paths, and daemon state machine.
- `gui/edgemap_gui/`: PyQt6 configuration editor driven by `edgemap capabilities`.

Read the relevant section of `docs/DEVELOPMENT_REFERENCE.md` before changing an unfamiliar subsystem.

## Core invariants

- Unknown or short source frames are dropped rather than raw-forwarded.
- Input order is L1 physical filtering, L2 virtual generation, then L3 target encoding.
- Remapping uses a frozen snapshot and two phases so cross-maps are order-independent.
- Target output decodes to a transport-neutral command before physical encoding.
- USB and Bluetooth report layouts stay in their protocol-specific codec modules.
- Physical sources may use USB or Bluetooth; virtual targets remain USB UHID devices.
- Edge-only buttons are valid sources but not gamepad targets.
- Configuration applies transactionally from bounded regular-file content; failed runtime applies preserve the previous live state.
- Required startup failures stop the daemon; individual output/feature failures do not stop the input path.
- GUI button, target, output-device, macro-name, and keyboard lists come only from the versioned capabilities contract.

## Detailed references

- `docs/DEVELOPMENT_REFERENCE.md`: architecture, pipeline, codecs, error behavior, quirks, configuration constraints, and migration notes.
- `docs/INSTALLATION_TESTING.md`: installation, packaging, release layout, and device-access operations.

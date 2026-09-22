# Installation and system testing

This document contains the system-impacting operational details intentionally kept out of the always-loaded `AGENTS.md` project instructions. Use these workflows only for installation, packaging, release, or device-access tasks that explicitly require them.

## Verification commands

The ordinary development workflow uses the unprivileged build and test commands in `AGENTS.md`.

The installer integration test is only for a disposable CI or test VM because it writes fixed system paths:

```bash
CI=true sudo --preserve-env=CI bash tests/test_install.sh target/debug
```

The root `PKGBUILD` is a VCS package for local testing. It does not require manual `pkgver` updates. Its local build-and-install command is:

```bash
makepkg -si
```

## Runtime and device-node access

- The `dseuhid` daemon requires root; builds, tests, and informational subcommands do not.
- `/run/dseuhid/control.sock` is a mode-0666 Unix `SOCK_SEQPACKET` endpoint.
- Permission hiding/restoration failures are warning-level unless a direct node restriction call returns an error to the caller.
- On suspend/resume, the daemon file descriptor survives but udev resets hidraw node permissions. `re_restrict_self()` checks mode `& 0o777` on the first post-resume input packet and reapplies `chmod 000` (#70, #71).
- Each hidden node is checked independently. If logind or udev reapplies a non-zero mode, `re_restrict_self()` snapshots the new mode and ACL before hiding it again; shutdown restores the latest snapshot (#91).

## Installer behavior

`install.sh` is a cwd-independent release installer/uninstaller with payload preflight, mandatory root, fixed system paths, direct file operations, and explicit systemd guidance. `DESTDIR` is intentionally unsupported.

Before replacing installed files, the installer runs both packaged binaries with
`--help` to verify that they can load. Missing shared libraries or incompatible
binaries abort installation with the original loader error. Required runtime
packages include `systemd-libs` on Arch or `libudev1` on Debian/Ubuntu. Building
requires the corresponding development files (`libudev-dev` on Debian/Ubuntu).

Bluetooth HD haptics and the experimental speaker path additionally need `pw-cat`
(`pipewire-audio` on Arch, `pipewire-bin` on Debian/Ubuntu) and a running user
PipeWire session. Missing `pw-cat` produces a warning without blocking installation
of the HID proxy. The installer does not install packages or enable the speaker
demo; `EDGEMAP_SPEAKER_DEMO=1` remains an explicit opt-in for `edgemap daemon`.

Opus is optional and used only by the speaker path: `opus` on Arch or `libopus0`
on Debian/Ubuntu. `edgemap` loads `libopus.so.0` dynamically when speaker capture
starts. Without it, normal edgemap startup still works; its HD haptics path does
not load Opus. An explicit speaker request fails with a dependency error instead
of silently falling back. The installed `pw-cat` must still have its own runtime
dependencies satisfied (on Arch, `libsndfile` indirectly requires Opus). No Opus
development package is needed to build. CI/release Rust tests install `libopus0`
to exercise the encoder/decoder; GUI-only jobs do not need it. The Arch package
lists both Opus and PipeWire as optional dependencies.

The `edgemap-gui` launcher resolves `/usr` or `/usr/local` from its own path and imports the private package from `<prefix>/lib/edgemap-gui`.

## Release tree

GitHub Actions builds this tarball layout on a tag:

```text
edgemap-v1.3.1-x86_64.tar.gz
├── install.sh                 # sudo ./install.sh
├── LICENSE                    # canonical GPLv3 license
├── dseuhid                    → /usr/local/bin/
├── edgemap                    → /usr/local/bin/
├── edgemap-gui                → /usr/local/bin/ (launcher)
├── usr/lib/systemd/
│   ├── system/dseuhid.service
│   └── user/edgemap.service
├── usr/local/lib/edgemap-gui/
│   └── edgemap_gui/           → private Python package
└── usr/share/
    ├── applications/edgemap.desktop
    ├── icons/hicolor/scalable/apps/edgemap.svg
    └── zsh/site-functions/{_dseuhid,_edgemap}
```

CI rewrites service files to use `/usr/local/bin/` paths at packaging time. The AUR `PKGBUILD` uses the repository defaults under `/usr/bin/`.

The release job verifies that the pushed tag matches the package version, runs formatting, locked Rust tests, Clippy with warnings denied, the Python 3.11 GUI suite, and the fixed-path installer test against the same release binaries passed to `scripts/stage_release.sh`. The staging script validates its payload, includes the repository `LICENSE` verbatim, and refuses to overwrite an existing output directory.

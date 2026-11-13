# edgemap user guide

## Controller compatibility

edgemap supports one DualSense or DualSense Edge over USB or Bluetooth. With **Auto** selected, features that work in a game over USB with official Proton should also work through edgemap. These include buttons, sticks, triggers, the D-pad, Edge paddles and Fn buttons, motion controls, touchpad input, rumble, HD haptics, LEDs, and adaptive triggers.

The built-in speaker and microphone may work in games over USB, but both lack in-game testing. Bluetooth speaker playback is experimental and also lacks in-game testing. Bluetooth microphone recording is unsupported.

> [!IMPORTANT]
> When using edgemap with Proton, many games need a patched Proton build for HD haptics or virtual DualShock 4 recognition. [proton-eg-patch](https://github.com/Kurobac/proton-eg-patch)

## Components and controller modes

`dseuhid` is the system service. It reads the physical controller, applies your mappings, and presents a virtual USB controller to the game. `edgemap` is the command-line tool. Its user service selects profiles and handles Bluetooth haptics audio. `edgemap-gui` is the editor for mappings, macros, and profile settings. You can close it while playing.

Choose the output model at the bottom of the editor. The choice is saved with each mapping.

- **Auto (match physical)** is the default. A DualSense appears as a DualSense and an Edge appears as an Edge.
- **DualSense** lets an Edge work with games that recognize only the regular model. Map its paddles and Fn buttons to ordinary buttons or keyboard keys to use them in those games.

To keep the grips playing audio haptics when a game sends ordinary rumble, check **Force audio haptics** in the editor or set `force_audio_haptics = true` at the top of the mapping file. Adaptive triggers and lights still respond to the game. You can apply this setting while playing. Clearing it gives the game control of the grip mode on its next output command. Existing mappings use game-controlled haptics by default.

Applying a different output model reconnects the virtual controller. Choose the model before launching games that detect controllers only at startup.

For games without native DualSense support, you can also use edgemap with Steam Input or a similar tool.

## DualShock 4 target (Beta)

For a game with native DualShock 4 support, select **DualShock 4 (Beta)** in the editor or set `output_device = "dualshock4"` in its mapping file. The game sees a USB DS4, and your mappings still apply. Buttons, sticks, triggers, touchpad input, lightbar, and ordinary rumble should work. For games that send native DualSense effects, choose **Auto** or **DualSense**. In DS4 mode, external audio sources can still play grip haptics through the controller audio device.

Some games under Proton look for a specific DS4 USB interface (`MI_03`). If the game ignores the virtual DS4, the DS4 identity patch supplies that interface identifier. [proton-eg-patch](https://github.com/Kurobac/proton-eg-patch) · [DS4 compatibility notes](DS4_TARGET_STATUS.md)

## Configuration and profiles

On first run, the user service creates two TOML files in `~/.config/edgemap/`. **A mapping file** such as `default.toml` says what each button does. **The profile settings** in `edgemap.toml` decide which mapping to use. In the GUI, edit bindings in the main window and open **edgemap** on the toolbar to edit profile settings.

For example, save this mapping as `game.toml` in the config directory. It maps the right Edge paddle to Space. Holding the left paddle and pressing Cross sends Triangle. Other buttons keep their usual behavior.

```toml
version = 2
output_device = "auto"

[right_paddle]
remap = "key:space"

[left_paddle]
remap = "combo"

[[left_paddle.combos]]
key = "cross"
output = "triangle"
```

Use lowercase button names. `remap` accepts a controller button, a stick direction such as `ls_up`, a full trigger press such as `r2_full`, a keyboard key, or a named macro. Use `block` to disable a button. Run `edgemap create-config` to print the annotated template with turbo, combo, macro, and split-touchpad syntax.

To use that mapping whenever a game runs, add a profile to `edgemap.toml`. Replace `example.exe` with text from your game's command line.

```toml
config = "default.toml"

[profiles.example]
config = "game.toml"
match_cmdline = "example.exe"
```

`match_cmdline` looks for a substring. `match_process` matches the whole Linux process name. Both ignore case. If you set both, the same process must satisfy both. The service picks the first matching profile in file order and returns to the base mapping when no profile matches.

**To try an edit immediately**, save the file, then run `edgemap switch-config game.toml`. A bare filename here refers to your edgemap config directory. Saving in the editor validates and writes the file; switching sends it to the running proxy. For a file edited by hand, run `edgemap validate /path/to/game.toml` to check it. A manual switch stays active until the service selects a different profile, reloads its profile settings, or reapplies a mapping after reconnection.

## Input processing order

The physical press goes through touchpad splitting, turbo, combo detection, and blocking, in that order. The remaining inputs drive remaps and macros. edgemap then combines their outputs with any active combo outputs. This order matters when bindings overlap.

- **Touchpad and turbo.** Split mode turns a touchpad click into a left or right click according to where your finger is. Configure both halves. Each can have its own mapping or turbo setting. Turbo alternates a held button between pressed and released, so a remap to a keyboard key can repeat too. `turbo_delay_ms` sets the initial delay and `turbo_interval_ms` sets the time between toggles.
- **Combos and blocking.** In the example above, Cross works normally on its own. With the left paddle held, the combo takes over Cross and produces Triangle. Blocking also acts on the physical button. A blocked Cross can still be produced by a mapping from another button. On an Edge, the firmware reserves Fn + face button for switching hardware profiles.
- **Swaps and shared outputs.** Map Cross to Circle and Circle to Cross and each press produces its opposite. Both rules read the same input before either writes its result. If two mappings hold Space, it stays down until both release it. For overlapping trigger outputs, the strongest requested value wins.
- **Macros.** A macro defines press and release times in milliseconds from the start of its sequence. Steps can overlap. With `single`, a tap plays the sequence to completion. With `hold`, the sequence loops while held and stops on release. A combo can trigger a macro by name.

## Bluetooth

When you connect over Bluetooth, edgemap presents the selected controller model to the game as a USB device. edgemap translates inputs, including motion and touch, into that controller's USB reports. In the other direction, it sends ordinary rumble, light changes, and adaptive-trigger commands back over Bluetooth. Vendor-specific feature writes used by some hardware-test tools are unsupported over Bluetooth.

The user service creates a four-channel PipeWire output named `edgemap.dualsense` in every Bluetooth output mode. Send haptic audio to its rear left and right channels. The output disappears when the controller disconnects. With a USB connection, use the physical controller's rear audio channels.

Bluetooth HD haptics require a running PipeWire session and `pw-cat` (`pipewire-audio` on Arch, `pipewire-bin` on Debian/Ubuntu). Experimental speaker playback uses the two front channels and also requires `libopus.so.0`.

By default, edgemap repeats the latest Bluetooth input at 1000 Hz for a virtual DualSense or Edge. This gives games a steady 1 ms interval between reports even when the physical controller’s reports arrive unevenly. The report sequence advances while the sensor timestamp stays with the last real sample. In Genshin Impact, this timing almost eliminated the visible gyro jitter seen with uneven report intervals. The DS4 mode follows incoming Bluetooth reports by default. Both behaviors can be adjusted in the [environment reference](#environment-variables).

## Troubleshooting

0. **Check the logs.** The system log shows controller connections and mappings accepted by the proxy. The user log shows profile selection and Bluetooth audio. To follow both while reproducing the problem, run these commands in separate terminals.

   ```sh
   journalctl -b -f -u dseuhid
   journalctl --user -b -f -u edgemap
   ```

   For debug output, run `sudo systemctl edit dseuhid` for the system service or `systemctl --user edit edgemap` for the user service. Add these lines to the override, then restart the edited service with `sudo systemctl restart dseuhid` or `systemctl --user restart edgemap`.

   ```ini
   [Service]
   Environment=RUST_LOG=debug
   ```

1. **Test the physical controller in the game.** Close the game and stop both services.

   ```sh
   systemctl --user stop edgemap
   sudo systemctl stop dseuhid
   ```

   Reconnect the controller and launch the game. Try the exact input or effect that failed, using the same connection (Bluetooth or USB) and Steam Input setting. For Bluetooth HD haptics, also try USB directly. The USB test establishes whether the game produces the effect that edgemap forwards over Bluetooth.

2. **Test the virtual controller.** Close the game and start both services again.

   ```sh
   sudo systemctl start dseuhid
   systemctl --user start edgemap
   ```

   Open dualsense-tester or Steam’s controller input test. Check that the virtual controller appears and that the affected input produces the expected response. For example, if a paddle is mapped to Cross, pressing it should register as Cross. [dualsense-tester](https://ds.daidr.me/)

3. **Select the patched Proton build for the game.** If HD haptics are missing or the game ignores the virtual DS4, check the game’s Steam compatibility settings. Select a build with the relevant proton-eg-patch fixes and relaunch the game. Use **Auto** or **DualSense** for HD haptics, or **DualShock 4 (Beta)** to test DS4 recognition. [proton-eg-patch](https://github.com/Kurobac/proton-eg-patch)

4. **Compare the results.** If a supported feature works directly but fails with edgemap, repeat the comparison with the same patched Proton build and game settings. If enabling edgemap consistently causes the failure, report it as an edgemap bug. Include the game, controller model, connection, output mode, Proton build, and logs from the failed attempt. If the same failure also occurs with the physical controller directly, troubleshoot the game, Proton, or controller connection first. That failure already occurs without edgemap.

## Environment variables

| System service (`dseuhid`) | Default | Effect |
| --- | --- | --- |
| `DSEUHID_BT_DS5_USB_REPEAT_MODE` | `seq_only` | For a Bluetooth source and DualSense/Edge output. `seq_only` advances the report sequence on each repeat. `seq_ts` advances the sensor timestamp too. `passthrough` disables fixed-rate repeats. |
| `DSEUHID_BT_DS5_USB_REPEAT_HZ` | `1000` | DualSense/Edge repeat rate, 1–2000 Hz. Used when fixed-rate repeats are enabled. |
| `DSEUHID_BT_DS4_USB_REPEAT_HZ` | Unset (repeat off) | Set to 1–2000 Hz to enable fixed-rate input for Bluetooth with DS4 output. |
| `DSEUHID_BT_HAPTICS_BUFFER` | `64` | Controller buffer setting for Bluetooth haptics-only audio, 1–255 in controller units. The combined speaker/haptics mode uses a fixed value of 64. |

| User service, GUI, and shared settings | Default | Effect |
| --- | --- | --- |
| `EDGEMAP_SPEAKER_DEMO` | Off | Set to `1` on the **edgemap user service** to enable experimental Bluetooth speaker playback. |
| `EDGEMAP_BINARY` | `edgemap` from `PATH` | CLI executable the **GUI** uses to get supported mappings and validate files. Accepts an executable name or path. |
| `RUST_LOG` | `info` | Log filter for either service. Use `debug` for more detail. |
| `RUST_LOG_STYLE` | `auto` | Log styling for either service. Accepts `auto`, `always`, or `never`. |
| `XDG_CONFIG_HOME` | `$HOME/.config` | Parent of the `edgemap/` config directory used by the CLI, user service, and GUI. Use an absolute path. |
| `XDG_STATE_HOME` | `$HOME/.local/state` | Parent of the `edgemap/` state directory used by the user service for its lock. Use an absolute path. |
| `HOME` | User's home directory | Supplies the fallback base for both XDG paths and expands `~/` in profile paths. |

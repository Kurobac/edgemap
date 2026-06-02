# v0.0.9 Refactor Notes

## Summary

v0.0.9 should be a pipeline refactor only. It should not add combo/macro runtime behavior, config parsing, or validation. The goal is to separate physical filtering from virtual generation so v0.0.10 can add combo/macro without another pipeline rewrite.

## Scope

- Move `remap = "block"` out of `RemapRule` / `Target`.
- Add `blocked_buttons` to `MappingConfig`.
- Move turbo handling before remap in the per-frame pipeline.
- Change `MappingConfig::apply()` to read an explicit frozen L1 state.
- Keep combo/macro as future TODO slots only.

Do not add `ComboRule`, `ComboOutput`, `remap = "combo"` parsing, macro collision checks, or combo/macro validation in v0.0.9.

## Intended Pipeline

```text
parse input report
  -> touchpad split
  -> turbo L1 update
  -> block suppression
  -> freeze L1
  -> remap virtual generation
  -> apply_state_to_report
  -> UHID input
```

Rules:

- Turbo reads the physical snapshot and writes to the live state before L1 is frozen.
- Block suppression clears the live state before L1 is frozen.
- Remap reads only the frozen L1 snapshot and writes virtual output to the live state.
- Downstream writes must not affect upstream decisions.

## Block Semantics

`remap = "block"` is physical suppression, not virtual generation.

Implementation requirements:

- `Target::Block` should be removed.
- `Config::to_mapping_config()` should put blocked sources into `MappingConfig::blocked_buttons`.
- Block suppression must clear the digital button.
- If the blocked source is `L2` or `R2`, block suppression must also clear `l2_analog` or `r2_analog`.

`turbo = true` plus `remap = "block"` is allowed. If configured alone, final output is empty because block wins the final L1 state.

## Remap Semantics

`MappingConfig::apply()` should become:

```rust
apply(&self, l1: &GamepadState, state: &mut GamepadState)
```

Rules:

- Source detection reads `l1`.
- Source clearing and target injection write `state`.
- The existing two-phase button target behavior should remain.
- Remap must not read outputs created by earlier remap rules or other L2 components.

## Future Combo Semantics

Future combo detection must happen before combo/block suppression affects the final L1 state.

The intended v0.0.10 model:

```text
physical snapshot
  -> turbo evaluation / post-turbo view
  -> combo detection reads post-turbo, pre-suppression view
  -> combo suppression clears consumed physical sources
  -> block suppression clears blocked sources
  -> freeze final L1
  -> combo injection / remap / macro virtual generation
```

Important behavior:

- Combo detection reads the post-turbo, pre-suppression view.
- Combo suppression and block suppression affect passthrough/remap input, not combo detection.
- Example: with `a+b=c` and `[b] turbo=true remap="block"`, holding `a+b` should output `c` following `b`'s turbo phase, while `a` and `b` themselves are not output.

This future behavior should not be implemented in v0.0.9; it is recorded here so the v0.0.9 pipeline does not block it.

## Config Notes

- Config section names are intended to be case-sensitive lowercase.
- The current `Button::from_name()` lowercases input, so `[Cross]` may currently pass validation.
- If v0.0.9 touches config validation, add a test that `[Cross]` is rejected as an unknown source.

## Tests To Add Or Update

- Update mapping tests for the new `apply(&l1, &mut state)` signature.
- Replace `Target::Block` tests with config/block suppression tests.
- Add a test that `remap = "block"` creates no `RemapRule` and adds the source to `blocked_buttons`.
- Add a trigger block test: `L2/R2` block clears both digital state and analog value.
- Add a validation test that `turbo = true` with `remap = "block"` is allowed.
- Add a behavior test that `turbo + block` alone produces no output.
- Add a config case-sensitivity test for uppercase section names.

## Non-goals

- Do not update old status/warning documentation unless needed for the refactor itself.
- Do not implement combo/macro state machines.
- Do not add a new runtime abstraction unless the existing `proxy.rs` flow becomes unreasonably hard to test.
- Do not commit without explicit approval.

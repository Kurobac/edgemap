# 配置验证阶段总结

## 允许

| 类别 | 详情 |
|------|------|
| **Source 按钮** | 标准按钮 + Edge 按钮（paddles/Fn），排除 `mic`、`l2_analog`、`r2_analog` |
| **Target** | 标准按钮（排除 Edge 按钮、Mic、TouchpadLeft/Right）+ 特殊 target: `l2_full`/`r2_full`、`ls_up/down/left/right`、`rs_up/down/left/right` + `combo` + 宏名称 |
| **remap="block"** | 禁用按钮（L1 层抑制） |
| **remap="combo"** | 组合键模式（须有 ≥1 个 combo 条目） |
| **turbo** | 按住重复触发（但不能和宏共存） |
| **宏** | `mode = "hold"`（循环）或 `"single"`（一次） |
| **touchpad split** | 触控板左右分区映射 |
| **无 remap** | passthrough，跳过不处理 |

## 禁止

- **未知名称** — source/target/宏 key 不认识就报错
- **`mic`、`l2_analog`、`r2_analog`** 不能当 source 也不能当 target
- **Edge 按钮**（paddles/Fn）不能当 target（只能当 source）
- **大写 Section** — `[Cross]` 不行，必须 `[cross]`
- **combo 相关：**
  - `remap="combo"` 但无 entries
  - 有 `combos` 但 `remap != "combo"`（互斥）
  - combo key 与 modifier 相同
  - combo key 是 `mic`/`l2_analog`/`r2_analog`/`touchpad_left`/`touchpad_right`
  - combo 重复 key
  - `FN` + `face buttons`（跟固件 profile 切换冲突）
  - `touchpad_left/right` 不能用 combo 模式
- **turbo + trigger 模拟** — L2/R2 不能 turbo 成自身
- **turbo + 宏互斥** — 同一按钮不能同时 turbo 和触发宏
- **宏名冲突** — 不能与标准按钮名 / 内置 target（如 `l2_full`）重名
- **宏 mode** — 必须是 `"hold"` 或 `"single"`
- **宏 sequence** — 不能空；step key 必须有效；`release_ms` 必须 > `press_ms`
- **split touchpad：**
  - 必须同时有 `touchpad_left` + `touchpad_right`
  - 分区不能 `remap="block"`
  - `touchpad_left/right` 不能单独出现（必须配合 `[touchpad] remap="split"`）

## 跳过（continue，不生成 remap rule）

| 条件 | 原因 |
|------|------|
| 无 `remap` 字段 | passthrough，不映射 |
| `turbo = true` 的按钮 | 跳过 remap rule，由 `build_turbo_configs()` 处理；但 combo 仍然处理 |
| `remap = "block"` | 加入 `blocked_buttons` 列表，不生成 remap rule |
| `remap = "combo"` | 跳过 remap rule，只构建 `ComboRule` 列表 |
| `[touchpad] remap = "split"` | 跳过常规处理，由 split 逻辑生成 `TouchpadLeft`/`TouchpadRight` 两个规则 |

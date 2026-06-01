# dseuhid 开发状态 (2026-06-01)

## 已实现

| 版本 | 分支 | 功能 |
|------|------|------|
| v0.0.1 | `main` (tag `0.0.1`) | 纯 passthrough UHID 代理 |
| — | `step1-remap` | passthrough + remap 集成（有 bug） |

## v0.0.1 功能确认

| 功能 | 状态 | 测试环境 |
|------|------|---------|
| 按键/摇杆/陀螺仪/触控板 | ✅ | Steam 手柄测试、KDE、WebHID tester |
| FN/背键 (DSE 专用) | ✅ | KDE、WebHID tester |
| LED / 玩家指示灯 | ✅ | WebHID tester output panel |
| 震动 | ✅ | CP2077、WebHID tester |
| 自适应扳机 | ✅ | CP2077 |
| HD haptics | ✅ | 原神 (原生 DS 游戏) |
| Steam Input 游戏 | ✅ | — |
| SDL 游戏 | ✅ | — |
| 热插拔 | ✅ | 拔→等重插→自动恢复 |
| Ctrl+C 退出 | ✅ | 运行时、等待重插时均可 |
| 1kHz 扬声器测试 | ❌ | WebHID tester — playstation 内核驱动不转发 report 0x80 |
| ACL/权限恢复 | ✅ | 拔线或退出时自动还原 |

## v0.0.1 commit 历史

```
fb39885 dseuhid: add hotplug support (poll-based)
8fbb7b8 step0: remove dead code from report.rs
373efce dseuhid: forward SET_REPORT data to real hardware via hidraw
56c6528 dseuhid: fix output report forwarding - check UHID rtype not HID report ID
450cc4d dseuhid: reply to SET_REPORT with UHID_SET_REPORT_REPLY
18e826e dseuhid: save and restore ACL when hiding device nodes
b5ad76a dseuhid: fix ACL restore - don't use setfacl -b when hiding
ed15311 dseuhid: init pure UHID proxy from old implementation
```

## step1-remap 分支 (remap 集成)

### 新增文件
- `src/mapping.rs` — `RemapRule { src, dst }`, `MappingConfig { rules }`, `apply()`
- `src/report.rs` — 新增 `apply_state_to_report(raw, state, seq)` / in-place 写入 / 保留 byte 11 低 4 位

### proxy.rs 改动
```
parse → mapping.apply → apply_state_to_report    [成功]
parse → buf[7]=*seq (fallback passthrough)       [失败]
```

### Bug: FN/背键失效

硬件 raw[11] 有信号，但虚拟设备输出中 FN/背键无反应。其余按键、摇杆、陀螺仪、映射规则均正常。

### 排查过程

1. byte 11 低 4 位被清零 → 修正 `b3 = raw[11] & 0x0F`。无效。
2. parse 失败缺少 fallback → 修正 fallback `buf[7]=*seq` + `send_input`。无效。
3. debug 日志 condition 不匹配 — 需加无条件诊断日志确认 raw[11] 值和 parse 返回值。

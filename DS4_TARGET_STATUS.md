# DS4 Target — 当前状态总结

## ✅ 已验证正常的部分

| 组件 | 验证方式 |
|------|---------|
| Raw input report 格式 | Windows reWASD 抓包逐字节对比，完全一致 |
| 按钮/摇杆/扳机 | Steam + evtest + 原神 hidraw 路径 |
| 触摸板（点击+坐标） | evtest + Steam 手柄测试 |
| LED 输出 | Steam 调整颜色 + 原神也能发 LED |
| SDL/evdev 路径 | sdl2-jstest + 所有 SDL 游戏 |
| 内核驱动 probe | UHID start/open 正常，device node 创建成功 |
| Feature report (0x02/0x12/0xA3) | 全部缓存，非零 MAC + 1:1 校准值 + firmware 版本 |


目标，让HIDRAW路径的游戏正常识别手柄。

## ❌ 仍在问题中的部分

### 类别 A — 完全不认的游戏 (CP2077 / TLOU)

**UHID 事件序列：**
```
GetReport rnum=0x12, 0xA3, 0x81, 0x02    ← kernel probe，全部缓存命中
SetReport rnum=0x13                        ← kernel driver 校准握手 (0x13, 23bytes)
Output rtype=1                             ← Proton/游戏发 LED 命令
GetReport rnum=0x12                        ← 反复查询 (~2s/次)
```

**Proton 日志确认：**
- `bus_main_thread creating hidraw device 054c:05c4 with usages 0001:0005`
- `sdl_add_device controller id 0, is_gamepad 1`
- `deliver_next_report ... input report length 64` — 投递正常
- Proton 层**没有任何 `write feature report`**（对比 DS5 passthrough 有 `rnum=8`）

**关键特征：**
1. 游戏**不发送任何 SET_REPORT**（对比 DS5 passthrough 有 `rnum=8`）
2. **不断重读 MAC (0x12)** — Proton 层认为设备身份不明确
3. 游戏发了 LED 命令 → 设备已被 open、能通信
4. 改名 `"Wireless Controller"` 后游戏行为有变化（kernel SetReport 触发）
5. `uniq` 填非空 MAC 后重读频率没有实质性减少

### 类别 B — 认手柄但 gyro/vibration 不工作 (原神)

- 显示了陀螺仪菜单 → 游戏按 VID/PID/Descriptor 识别为 DS4
- 按键/摇杆正常 → input report 字节正确
- gyro/vibration 不工作 → evtest 验证 gyro 数据正确、Output 震动转换在 Steam 验证过

可能原因：
- gyro 校准值和游戏预期不一致（手工 1:1 构造，非真 DS4 值）
- vibration 走的路径需要 SET_REPORT 握手先完成（和类别 A 同一根因）

## 🔍 已排除的原因

| 原因 | 验证 |
|------|------|
| HID report 格式错误 | reWASD Windows 逐字节对比完全一致 |
| Feature report 缺失 (0x04-0x15) | reWASD 也是 error |
| MAC 地址全零 | 已改为 `C0:13:37:00:00:01` |
| 设备名称 | 已改为 `"Wireless Controller"` |
| SET_REPORT 转发 broken pipe | 已跳过 DS4 下的物理转发 |
| `HIDIOCGRAWUNIQ` 为空 | 已填 `"c0:13:37:00:00:01"` |
| PID 版本 (0x05C4 vs 0x09CC) | v2 下的游戏没测过 |

## 💡 最可能的根因

**游戏在比 SET_REPORT 更早的阶段就决定"不用这个 DS4"**。

DS5 passthrough 模式下游戏发 `SET_REPORT rnum=8` 作为初始化握手。
DS4 模式下**没有任何来自游戏的 SET_REPORT**——游戏在枚举阶段就排除了设备。

可能的卡点：
- `HIDIOCGDEVINFO` ioctl 返回的 version (Proton 显示 `version 0000`)
- `HIDIOCGRAWPHYS` 返回的 phys 路径
- HID descriptor 解析出的 caps 数量不符预期
- 游戏 hardcoded 的 DS4 controller PID 白名单

## 🎯 Windows API Monitor 抓到的游戏初始化序列 (CP2077)

| # | API | Report | Size | Result |
|---|-----|--------|------|--------|
| 1 | HidD_GetFeature | 0x12 (MAC) | 16B | TRUE |
| 2 | HidD_GetFeature | 0xA3 (firmware) | 49B | TRUE |
| 3 | **HidD_SetFeature** | **0x14 (init)** | **17B** | **TRUE** |
| 4 | HidD_GetFeature | 0x02 (calibration) | 37B | TRUE |

游戏在查完 0xA3 后发 SetFeature(0x14) 初始化手柄。这是 Linux 上缺失的关键步骤。

## 🔴 重大 Bug 发现并修复：Feature Report 数据被 report ID 填满

```rust
// ❌ bug: vec![0x12u8; 16] 把所有 16 字节填成 0x12
//    buf.resize(16, 0) 是 no-op (size 已=16)
//    只有 buf[1..7] 被 MAC 覆盖，其余字节全是 0x12 垃圾
let mut buf = vec![0x12u8; 16];
buf.resize(16, 0);
```

修复后：`vec![0u8; SIZE]` + `buf[0] = report_id`，其余为零。

影响：0x02、0x12、0xA3 三个报告全部修复。

## 🔴 HID Descriptor Report Count Bug 修复

HID 规范：Report Count 不含 report ID 字节。PSDevWiki 的数字是总大小（含 ID），直接填入导致 Proton 解析出错误 caps。

| Report | 修复前(错) | 修复后(正) | 内核硬编码(含ID) |
|--------|-----------|-----------|-----------------|
| 0x02 cal | 0x25(37) | 0x24(36) | 37 |
| 0x12 MAC | 0x10(16) | 0x0F(15) | 16 |
| 0xA3 fw | 0x31(49) | 0x30(48) | 49 |

修复后 MAC 不再被 Proton 无限重读（20+ 次 → 1 次）。

## 🔍 Linux strace vs Windows API Monitor 对比

| | Windows (API Monitor) | Linux (strace) |
|---|---|---|
| Open /dev/hidraw ×2 | N/A | ✅ PID 287869 + 287883 |
| HIDIOCGFEATURE(0x12) → 16B | ✅ Thread 73 | ✅ fd 55 & fd 68 |
| HIDIOCGFEATURE(0xA3) → 49B | ✅ Thread 77 | ❌ **不存在** |
| HIDIOCSFEATURE(0x14) → 17B | ✅ Thread 77 | ❌ **不存在** |
| HIDIOCGFEATURE(0x02) → 37B | ✅ Thread 77 | ❌ **不存在** |

Linux 上两次 HIDIOCGFEATURE 都查询 0x12 且都返回 16B——Thread 77 从未查询 0xA3。
两个进程（steam + winedevic）各 open 一次 hidraw11，共享同一设备。

## 🔁 已排除的因素

| 假设 | 证据 |
|------|------|
| Raw report 格式错误 | ✅ reWASD 逐字节对比一致 |
| Feature report 数据内容 | ✅ 已修复 report ID 填充 bug，0x12 返回干净数据 |
| HID Descriptor Report Count | ✅ 三个关键报告已修复 |
| Version Number (HID Attributes) | ✅ reWASD 返回 0x0100，我们也是 0x0100 |
| Proton 白名单 | ✅ 源码确认 is_dualshock4_gamepad() 返回 TRUE |
| 0x12 数据被 Proton 截断 | ✅ strace 确认 ioctl 返回 16B |
| 设备名称 | ✅ 改为 "Wireless Controller" |
| MAC 地址全零 | ✅ 改为 C0:13:37:00:00:01 |
| FIRMWARE 版本 | ✅ 改为真实 DS4 值 0x0049 |

## ⏳ 待确认：真 DS4 硬件对比

已购买真 DS4 硬件，到货后用同一 strace 抓取初始化序列，对比：
- HIDIOCGFEATURE 查询顺序和返回字节数
- 是否有 HIDIOCSFEATURE (SetFeature 0x14)
- 游戏完整初始化流程和我们的差异

**工具：** [API Monitor](http://www.rohitab.com/apimonitor)（免费，< 2MB）

**监听 API：**
```
Data Access:         ReadFile, WriteFile
Device I/O:          DeviceIoControl
HID:                 HidD_GetFeature, HidD_SetFeature,
                     HidD_GetInputReport, HidD_SetOutputReport,
                     HidD_GetAttributes, HidD_GetPreparsedData
```

**关注点：**
- `HidD_GetAttributes` → VersionNumber 返回值
- `HidD_GetPreparsedData` → 解析后的 caps 数量
- `HidD_SetFeature / HidD_GetFeature` → 初始化握手顺序和参数
- `ReadFile` → 第一次成功读取的时间点和数据大小

## 📊 已实现功能清单

| 功能 | 状态 | 文件 |
|------|------|------|
| DS4 HID 描述符 (467 bytes) | ✅ | `src/descriptor.rs` |
| Input report 写入 (DS4 64-byte layout) | ✅ | `src/report.rs:apply_state_to_ds4_report()` |
| Output report 转换 (DS4→DS5 rumble+LED) | ✅ | `src/report.rs:convert_ds4_output_to_ds5()` |
| 触摸板坐标转发 (Y轴 1080→942) | ✅ | `src/report.rs` (fill(0) before snapshot) |
| 陀螺仪/加速度计转发 (1:1 calibration) | ✅ | `src/report.rs` + `src/main.rs` (cal data) |
| Feature report 缓存 (0x02/0x12/0xA3) | ✅ | `src/main.rs` |
| 固件版本 (hw=0x0001, fw=0x0100) | ✅ | `src/main.rs` |
| 非零 MAC (C0:13:37:00:00:01) | ✅ | `src/main.rs` |
| 设备名 "Wireless Controller" | ✅ | `src/main.rs` |
| 非空 uniq 字符串 | ✅ | `src/main.rs` |
| Output device 枚举 (auto/dualsense/dualshock4) | ✅ | `src/config.rs` |
| GUI Output Device 下拉菜单 | ✅ | `edgemap-gui-v6.py` |
| UHID reload 检测 output_device 变化 | ✅ | `src/proxy.rs` |
| 传感器温度 (byte 12=0x06) | ✅ | `src/report.rs` |
| 电池状态 (byte 30=0x1B) | ✅ | `src/report.rs` |
| 触摸点 INACTIVE 标志 (0x80) | ✅ | `src/report.rs` |

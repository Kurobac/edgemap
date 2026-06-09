# DS4 Target — 当前状态总结

## ✅ 已验证正常的部分

| 组件 | 验证方式 |
|------|---------|
| DS4 HID 描述符 (467 bytes) | 与 ViGEmBus 逐字节一致；与 reWASD Caps 完全匹配 |
| Raw input report 格式 | Windows reWASD 抓包逐字节对比，完全一致 |
| 按钮/摇杆/扳机 | Steam + evtest 正确 |
| 触摸板（点击+坐标） | evtest + Steam 手柄测试 |
| DS Edge passthrough 模式 | 正常工作（游戏认手柄） |
| DS5 forced 模式 | 正常工作（游戏认手柄） |
| 内核驱动 probe | UHID start/open 正常，device node 创建成功 |
| Feature report 数据 (0x02/0x12/0xA3) | Wine 测试程序验证全部四步可正常调用 |
| Wine HID 转发层 | 测试程序确认 HidD_GetFeature(0x12/0xA3/0x02) + HidD_SetFeature(0x14) 全部成功 |

## ❌ 仍在问题中的部分

### HIDRAW 路径游戏完全不认 (CP2077 / TLOU)

通过 `WINEDEBUG=+hid` 对比 DS4（不工作）vs DS5 forced（工作），两者行为分叉点精确定位：

```
DS4: open → SonyIOCTL → attributes → collection
     open → attributes → deepInspect → GetFeature(0x12) → close   ← 仅读串号，中止
     open → attributes → deepInspect → close                       ← 后续仅轮询

DS5: open → SonyIOCTL → attributes → collection
     open → attributes → deepInspect → GetFeature(0x09) → GetFeature(0x20) → close  ← 串号+固件
     open → attributes → deepInspect → close
```

DS4 在 Linux 下只读 0x12（串号），不读 0xA3（固件），不发 SET_REPORT；DS5 读两个 feature report。DS Edge (auto) 和 DS5 forced 均正常工作。

### DeepInspect 三路对比（已排除）

用 `tools/hid-inspect.exe` 在三个环境对比了游戏 deepInspect 阶段的精确 API 返回值：

```
                         reWASD DS4 (Win,认)    dseuhid DS4 (Linux)   dseuhid DS5 (Linux,认)
LinkCollectionNodes      count=1 nt=0x110000     count=1 ✅              count=1 ✅
OutputBtn(0x0F,0x9A)     count=0 nt=0xc0110004   count=0 ✅              count=0 ✅
InputBtn(Button,0)       count=1 min=1 max=14    count=1 ✅              count=1 min=1 max=15
InputVal(X=0x30)         count=1 nt=0x110000     count=1 ✅              count=1 ✅
InputVal(Y=0x31)         count=1 nt=0x110000     count=1 ✅              count=1 ✅
InputVal(0x02,0xC8)      count=0 nt=0xc0110004   count=0 ✅              count=0 ✅
```

**每一项的 ntstatus 和 count 三路完全相同。deepInspect 不是分叉点。**

## 🔍 详细排查历程

### Wine/Proton HID 转发层验证

编写了 Windows 测试程序 (`tools/hid-inspect.exe` + `tools/ds4-probe.c`)，在 Wine 下模拟游戏的四步 DS4 初始化，全部成功。**Wine 层能完美转发全部调用，游戏主动选择不走完整初始化。**

### 游戏 HID 调用链（WINEDEBUG=+hid trace）

- DS Edge (auto) 和 DS5 (forced) 均正常工作
- DS4 模式下游戏仅读 0x12，之后不再调用任何 GET_FEATURE/SET_FEATURE
- 游戏发 Sony 私有 IOCTL `0xb04b0` (HID_BUFFER_CTL_CODE(300)) 14 次探测，Wine 返回 STATUS_NOT_SUPPORTED——DS4/DS5 都收到这个结果，但 DS5 继续正常
- DS5 PID + DS4 描述符 hack 失败：hid-playstation 内核驱动因 PID/描述符不一致导致输出报告解析错乱

### Feature Report 数据修复历史

| 修复项 | 改动 | 文件 |
|--------|------|------|
| 0x12 MAC 字节序 | 反转序匹配 ViGEmBus/reWASD | `src/main.rs` |
| 0x12 bytes 7-9 | 填入 `0x08 0x25 0x00` (USB 就绪状态) | `src/main.rs` |
| 0x12 host MAC (10-15) | 清零匹配 reWASD (USB 连接无须 host MAC) | `src/main.rs` |
| 0xA3 firmware 字符串 | 填入 `"Aug  3 2013"` + `"07:01:12"` 匹配 ViGEmBus | `src/main.rs` |
| 0xA3 hw_version offset | 修正 off-by-one | `src/main.rs` |
| 0xA3 extra fields | 填入 0x0331/0x05/0x0380 | `src/main.rs` |

### Version Number 问题

Wine 下 `HidD_GetAttributes.VersionNumber` 始终为 `0000`。原因是 UHID 设备的 parent chain 中没有带 `PRODUCT=` uevent 行的 "usb"/"input" ancestor。内核实际存储的 version (`/sys/class/input/input*/id/version`) 为 `0x8100`，但 Wine 的 `bus_udev.c` 不读取。DS5 forced 同样 Version=0000 但正常工作。此问题除给Wine打补丁之外无解决办法，且是根本原因可能性较小。

### HID Descriptor Caps 对比

dseuhid DS4 与 Windows reWASD DS4 的 `HidP_GetCaps` 返回值完全一致：InputReportByteLength=64, OutputReportByteLength=32, FeatureReportByteLength=64, NumberInputButtonCaps=1, NumberInputValueCaps=9, NumberFeatureValueCaps=43。

## 💡 当前根因假设

DS4 的 DeepInspect 返回值与 Windows reWASD 完全相同，deepInspect 非分叉原因。**分叉在 PID 层面**——游戏看到 `PID=05C4` 进入 DS4 SDK 路径，`PID=0CE6` 进入 DS5 路径。

DS4 路径在 Windows 下正常工作，在 Linux/Proton 下中止。三路 deepInspect 完全一致后，真正的差异只剩：

| 差异 | Linux dseuhid DS4 | Windows reWASD DS4 |
|------|-------------------|---------------------|
| VersionNumber | **0000** | **0100** |
| 0x12 MAC 地址 | `01 00 00 37 13 c0` | `01 a1 f3 49 2c 72` |
| 0x02 校准数据 | 1:1 neutral (±1024/8192) | 真实 DS4 值 (不对称) |
| Manufacturer | "hidraw" | "Sony Computer Entertainment" |

**最可能触发游戏 DS4 路径中止的：`VersionNumber == 0x0000`。**DS5 路径不检查 version，DS4 路径检查。Version=0 被 SDK 解读为"非真硬件/virtual"并跳过后续初始化。

## ⏳ 下一步

1. **真 DS4 硬件对比**（已购买）：同一系统、同一 Proton 版本测试真 DS4，确认是虚拟化特有问题还是 Proton DS4 兼容性问题
2. **Version 注入实验**：尝试让 Wine 返回非零版本号（修改 `bus_udev.c` 添加 sysfs version 读取 fallback，或使用 LD_PRELOAD 拦截 ioctl）

## 📊 已实现功能清单

| 功能 | 状态 | 文件 |
|------|------|------|
| DS4 HID 描述符 (467 bytes) | ✅ | `src/descriptor.rs` |
| Input report 写入 (DS4 64-byte layout) | ✅ | `src/report.rs:apply_state_to_ds4_report()` |
| Output report 转换 (DS4→DS5 rumble+LED) | ✅ | `src/report.rs:convert_ds4_output_to_ds5()` |
| 触摸板坐标转发 (Y轴 1080→942) | ✅ | `src/report.rs` |
| 陀螺仪/加速度计转发 (1:1 calibration) | ✅ | `src/report.rs` + `src/main.rs` |
| Feature report 缓存 (0x02/0x12/0xA3) | ✅ | `src/main.rs` |
| 固件信息 + MAC 地址 | ✅ | `src/main.rs` |
| Output device 枚举 (auto/dualsense/dualshock4) | ✅ | `src/config.rs` |
| GUI Output Device 下拉菜单 | ✅ | `edgemap-gui-v6.py` |
| UHID reload 检测 output_device 变化 | ✅ | `src/proxy.rs` |
| 传感器温度 + 电池状态 + 触摸点 INACTIVE | ✅ | `src/report.rs` |
| 诊断工具 (hid-inspect, ds4-probe) | ✅ | `tools/` |

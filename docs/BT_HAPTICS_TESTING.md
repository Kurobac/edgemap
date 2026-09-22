# Bluetooth 音频触觉测试

本阶段验证四声道音频 → 后两声道 PCM → 蓝牙 HD 震动。需要本机用户会话中的
PipeWire 和 `pw-cat`；播放测试脚本另需 `pw-play`、Python 3。声卡名称随输出型号为
`DualSense Wireless Controller` 或 `DualSense Edge Wireless Controller`，节点名为 `edgemap.dualsense`，格式为四声道 48 kHz。

## 运行当前构建

控制协议已经升级到 v3，`dseuhid` 与 `edgemap` 必须使用同一次构建。
从仓库根目录执行：

```bash
cargo build
systemctl --user stop edgemap
sudo systemctl stop dseuhid
```

如果之前手动运行了代理，先在对应终端按 Ctrl+C。然后分别在两个终端启动：

```bash
# 终端一：物理 HID 代理
sudo ./target/debug/dseuhid
```

```bash
# 终端二：当前用户的 PipeWire 音频入口与配置 daemon
./target/debug/edgemap daemon
```

蓝牙连接并就绪后，检查：

```bash
pactl list short sinks
```

应出现 `edgemap.dualsense`、`4ch 48000Hz`。物理 USB 连接不创建此虚拟声卡。

`pactl list sinks` 中该节点还应提供 `device.bus = "usb"`、
`device.vendor.id = "0x054c"` 和 `device.form_factor = "controller"`。
Edge 的 auto 输出应使用 `device.product.id = "0x0df2"` 和 Edge 名称；
强制 `output_device = "dualsense"` 后应使用 `0x0ce6` 和普通 DualSense 名称。
改变输出型号会重建声卡，节点名仍为 `edgemap.dualsense`。

## 播放音频 demo

独立测试没有游戏来设置音频触觉模式，先运行之前的 HID demo；等它结束后，
再播放音频 demo：

```bash
./target/debug/edgemap haptics-demo
sleep 3
python3 scripts/haptics_audio_demo.py
```

第二段测试实际经过 PipeWire sink，不走 HID demo 的波形生成器。
其音频顺序为：前两声道 0.5 秒、后左 1 秒、静音 0.5 秒、后右 1 秒、静音收尾。
因此手柄应先保持静止，再左侧震动、暂停、右侧震动并停止。前两声道在本阶段丢弃。

播放期间断开蓝牙，声卡应消失；重新连接后应重新出现并可再次播放。
退出 `edgemap daemon` 也应移除声卡。测试完成后，在两个手动 daemon 终端按 Ctrl+C；
如需恢复安装版服务，再运行 `sudo systemctl start dseuhid` 和
`systemctl --user start edgemap`。

音频流本身保留游戏通过 HID 设置的模式。Sony USB 总线、VID/PID 和名称属性已经提供；ContainerId
关联仍待后续处理；本测试脚本通过节点名明确指定音频目标。

## 自动验证

普通 Rust 测试覆盖后声道提取、3 kHz 输出数量、抗混叠滤波、PCM 数据报校验、
队列上限、迟到丢帧、静音停流以及定时器到 HID report 的路径：

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

另有一个默认跳过、需要实际 PipeWire 用户会话的集成测试。它依次建立普通 DualSense 和 Edge 临时声卡，检查 PulseAudio 读回的型号、名称、
USB 总线、VID/PID 和 controller 分类，再播放四声道测试音频，检查 PCM 左右
声道和静音，以及退出后的声卡销毁：

```bash
cargo test --bin edgemap pipewire_quad_capture_to_pcm_and_sink_cleanup -- --ignored --nocapture
```

接收端关闭竞态的回归测试会在捕获线程仍运行、尚未收到停止通知时关闭 PCM
接收端，确认旧线程正常退出并移除声卡：

```bash
cargo test --bin edgemap pipewire_receiver_close_ends_capture_cleanly -- --ignored --nocapture
```

此集成测试不使用物理手柄，实际震感及蓝牙断开/重连仍需上面的实机测试。

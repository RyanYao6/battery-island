# 电量岛 · Battery Island

Rust + egui 写的 2.4G 无线外设电量小工具，Windows 托盘常驻，点开托盘即可看到设备剩余电量。

内置支持 **迈从 MCHOSE L7 Pro**（接收器 `5253:1020`），其它设备可通过配置文件添加。

> 非官方工具，与设备厂商无关。程序只发送读取命令，不写入 DPI、灯效、按键或任何配置。

## 功能

- 托盘常驻：左键显示 / 隐藏面板，右键菜单刷新与退出
- 电池形托盘图标，按最低电量取色（绿 / 橙 / 红），按 DPI 以 64px 生成
- 浅色圆角面板：设备图标、名称、型号、百分比、进度条、连接状态
- 面板高度随设备数自适应（1 台 336px、3 台 600px，最多同时显示 5 台）
- 设置以内嵌页切换，风格与面板一致，不遮挡卡片
- 窗口失焦自动隐藏；不占任务栏与 Alt-Tab
- 后台每 30 秒刷新，不阻塞界面；设备休眠时保留上次读数与时间
- 悬停卡片可查看 VID/PID、固件、原始响应帧等诊断信息
- 中文字体自动从系统加载，图标全部运行时绘制，无外部图片

## 构建

环境：Windows 10 / 11、Rust 1.100+、Visual Studio Build Tools（「使用 C++ 的桌面开发」+ Windows SDK）。

```bash
cargo build --release
copy config.toml target\release\
target\release\battery-island.exe
```

> Git Bash 下 Git 自带的 `link` 会抢在 MSVC 的 `link.exe` 前面，导致
> `link: extra operand`。编译前把 `VC\Tools\MSVC\<版本>\bin\Hostx64\x64` 放到
> PATH 最前，或直接用「x64 Native Tools Command Prompt」。

## 命令行

| 参数 | 说明 |
| --- | --- |
| 无参数 | 托盘常驻 |
| `--demo` | 用示例数据预览界面，不访问硬件 |
| `--read` | 读取一次电量并写出报告 |
| `--dump` | 导出候选 HID 接口与原始 Feature Report |
| `--help` | 显示帮助 |

`--read` / `--dump` 的报告写在 exe 同目录，便于换固件后核对字节。

## 配置

`config.toml` 放在程序同目录：

```toml
refresh_secs = 30      # 自动刷新间隔（秒，最小 5）
auto_hide = true       # 失焦自动隐藏
show_on_start = true   # 启动时显示一次面板
demo = false           # 演示模式
```

可追加 `[[extra_devices]]`（`vid` / `pid` / `usage_page` / `name` / `model`）让程序尝试其它设备。
日志位于 `%LOCALAPPDATA%\BatteryIsland\app.log`。

## 数据格式

读取走 HID Feature Report：Report ID `0x11`、命令 `0x06`、65 字节帧，载荷逐字节取反。

```text
请求：11 F9 FF×63
响应：11 F9 <payload ^ 0xFF>

payload[0..2]  vid        payload[2..4]  设备 pid
payload[4..8]  固件       payload[8]     连接状态
payload[9]     电量 %     payload[10]    充电状态
```

部分固件的报告长度是 21 字节，两种长度都会尝试。

设备空闲时会把写入的帧原样回显，读取需要成对 Set / Get 并重试；休眠时面板显示
「未连接」，唤醒设备后自动恢复。

## 常见问题

| 现象 | 处理 |
| --- | --- |
| 一直显示「未连接」 | 唤醒设备；退出厂商驱动（如 M HUB）后点「重新扫描」 |
| 电量长时间不变 | 设置里调短刷新间隔，或点刷新按钮 |
| 其它型号读不到 | 用 `--dump` 查看原始报告，按实际字节调整 |

## License

[MIT](LICENSE) © 2026 RyanYao6

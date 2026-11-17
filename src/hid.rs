//! 2.4G 无线外设电量读取。
//!
//! 目前支持两套协议：
//!
//! **迈从 MCHOSE L7 Pro（HID Feature Report）**
//!
//! ```text
//! 请求：11 F9 FF×63              命令与填充字节逐字节取反
//! 响应：11 F9 <payload ^ 0xFF>
//!
//! payload[0..2]  vid u16 LE      payload[2..4]  设备 pid u16 LE
//! payload[4..8]  固件 u32 LE     payload[8]     连接状态标志
//! payload[9]     电量百分比      payload[10]    充电状态
//! ```
//!
//! **狼蛛 AULA F98 Pro（2.4G 接收器，中断 IN/OUT 报告）**
//!
//! ```text
//! 请求（33 字节，Report ID 0x00）：00 20 01 00 00 …
//! 响应（33 字节）：              20 01 00 64 00 … 85
//!                                └ 帧头  └ 电量 └ 校验和 = 前面所有字节之和
//! ```
//!
//! 接收器没有任何 Feature Report，只能走输出报告 + 输入报告。
//! 电量取帧头之后的第 3 个字节（0x64 = 100%），范围 0..=100。
//!
//! 本模块只发送读取命令，不写入 DPI / 灯效 / 配置，不会改动设备设置。

use hidapi::{DeviceInfo, HidApi, HidDevice};
use std::ffi::CString;
use std::thread;
use std::time::Duration;

/// 短命令报告的 Report ID。
pub const REPORT_SHORT: u8 = 0x11;
/// 读取设备身份 / 电量的命令字。
pub const CMD_DEVICE_STATUS: u8 = 0x06;
/// 实机报告描述符给出的 Feature Report 长度（含首字节 Report ID）。
pub const FEATURE_LEN_NATIVE: usize = 65;
/// 兼容长度：20 字节载荷 + 1 字节 Report ID。
pub const FEATURE_LEN_COMPAT: usize = 21;

/// 已知厂商 ID。
pub const KNOWN_VIDS: [u16; 2] = [0x5253, 0x3837];
/// 厂商自定义集合的 Usage Page。
pub const VENDOR_USAGE_PAGES: [u16; 2] = [0xFF01, 0xFF0B];
/// L7 Pro 及同系列接收器常见 PID。
pub const L7_FAMILY_PIDS: [u16; 5] = [0x00B0, 0x1020, 0x1021, 0x1019, 0x1024];

/// 狼蛛 2.4G 接收器（F98 Pro 等）的 VID/PID。
pub const AULA_VID: u16 = 0x05AC;
pub const AULA_PIDS: [u16; 1] = [0x024F];
/// AULA 接收器的 raw HID 集合（32 字节报文 + Report ID）。
pub const AULA_USAGE_PAGE: u16 = 0xFF60;
/// 查询帧总长度（含 Report ID）。
pub const AULA_REPORT_LEN: usize = 33;
/// 电量查询的帧头。
const AULA_CMD: [u8; 2] = [0x20, 0x01];

/// 设备使用的电量协议。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// 迈从特征报告 0x11 / 0x06。
    Feature,
    /// 狼蛛 2.4G 接收器的中断报告查询。
    AulaRaw,
}

/// 用户通过配置文件额外声明的设备。
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ExtraDevice {
    pub vid: u16,
    pub pid: u16,
    #[serde(default)]
    pub usage_page: Option<u16>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// 一个可用于尝试通信的 HID 接口。
#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: CString,
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub interface: i32,
    pub product: String,
    pub serial: String,
    /// 自定义设备名字（来自配置），用于多设备区分。
    pub display_name: Option<String>,
    pub display_model: Option<String>,
    /// 排序权重：越小越优先。
    pub priority: u8,
    /// 该接口使用哪套电量协议。
    pub protocol: Protocol,
    /// 是否按键盘显示（图标 / 文案）。
    pub keyboard: bool,
}

impl Candidate {
    pub fn label(&self) -> String {
        format!(
            "{:04X}:{:04X}  page=0x{:04X} usage=0x{:04X} if={}  {}",
            self.vid, self.pid, self.usage_page, self.usage, self.interface, self.product
        )
    }

    pub fn key(&self) -> String {
        format!(
            "{:04x}:{:04x}:{}",
            self.vid,
            self.pid,
            self.path.to_string_lossy()
        )
    }
}

fn name_of(info: &DeviceInfo) -> String {
    let s = info.product_string().unwrap_or_default().trim().to_string();
    if s.is_empty() {
        "MCHOSE Wireless Receiver".to_string()
    } else {
        s
    }
}

/// 枚举所有可能与 L7 协议对话的 HID 集合。
///
/// 严格条件：已知 VID + 厂商 Usage Page；
/// 宽松条件：已知 VID + L7 族 PID（某些 Windows 版本对 vendor collection
/// 上报的 usage_page 不准确），或配置文件显式声明的 VID/PID。
pub fn find_candidates(api: &HidApi, extra: &[ExtraDevice]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for info in api.device_list() {
        let vid = info.vendor_id();
        let pid = info.product_id();
        let page = info.usage_page();
        let product = name_of(info);

        let extra_match = extra
            .iter()
            .find(|e| e.vid == vid && (e.pid == pid || e.pid == 0));

        // ---- 狼蛛 AULA 接收器：只有中断报告，没有 Feature Report ----
        if vid == AULA_VID && AULA_PIDS.contains(&pid) && page == AULA_USAGE_PAGE {
            out.push(Candidate {
                path: info.path().to_owned(),
                vid,
                pid,
                usage_page: page,
                usage: info.usage(),
                interface: info.interface_number(),
                product,
                serial: info.serial_number().unwrap_or_default().trim().to_string(),
                display_name: Some("狼蛛 F98 Pro".to_string()),
                display_model: Some("F98 Pro · 2.4G".to_string()),
                // 32 字节报文在 interface 3；interface 4 是 64 字节的另一套集合，排后面。
                priority: if info.interface_number() == 3 { 0 } else { 6 },
                protocol: Protocol::AulaRaw,
                keyboard: true,
            });
            continue;
        }

        let vendor_known = KNOWN_VIDS.contains(&vid);
        let page_ok = VENDOR_USAGE_PAGES.contains(&page)
            || extra_match.and_then(|e| e.usage_page).map_or(false, |p| p == page);
        let pid_ok = L7_FAMILY_PIDS.contains(&pid);

        if !vendor_known && extra_match.is_none() {
            continue;
        }
        if !page_ok && !pid_ok && extra_match.is_none() {
            continue;
        }

        // 实测：配置集合是 UsagePage 0xFF01（mi_02&col02），
        // 0xFF0B 是同一接收器的另一个集合，不响应 0x06，排在后面。
        let priority = if page == 0xFF01 && pid_ok {
            0
        } else if page_ok && pid_ok {
            1
        } else if page_ok {
            2
        } else if extra_match.is_some() {
            3
        } else {
            4
        };

        out.push(Candidate {
            path: info.path().to_owned(),
            vid,
            pid,
            usage_page: page,
            usage: info.usage(),
            interface: info.interface_number(),
            product,
            serial: info.serial_number().unwrap_or_default().trim().to_string(),
            display_name: extra_match.and_then(|e| e.name.clone()),
            display_model: extra_match.and_then(|e| e.model.clone()),
            priority,
            protocol: Protocol::Feature,
            keyboard: false,
        });
    }
    out.sort_by_key(|c| (c.priority, c.vid, c.pid, c.interface));
    out
}

/// 一次成功读取的结果。
#[derive(Clone, Debug)]
pub struct Reading {
    pub vid: u16,
    pub pid: u16,
    pub firmware: u32,
    pub flags: u8,
    pub battery: u8,
    pub charging: u8,
    pub inverted: bool,
    /// 实际生效的 Feature Report 总长度（含 Report ID）。
    pub frame_len: usize,
    pub label: String,
}

/// 按设备返回的固定布局解析载荷（前 11 字节有效）。
///
/// ```text
/// [0..2]  vid u16 LE      [2..4]  pid u16 LE
/// [4..8]  fwVersion u32 LE
/// [8]     bit3 connectMode | bit1 connectStatus | bit4 reserved
/// [9]     batteryLevel     [10]   chargeStatus
/// ```
pub fn parse_payload(d: &[u8]) -> Option<(u16, u16, u32, u8, u8, u8)> {
    if d.len() < 11 {
        return None;
    }
    let vid = u16::from_le_bytes([d[0], d[1]]);
    let pid = u16::from_le_bytes([d[2], d[3]]);
    let fw = u32::from_le_bytes([d[4], d[5], d[6], d[7]]);
    let flags = d[8];
    Some((vid, pid, fw, flags, d[9], d[10]))
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn request_frame(rid: u8, inverted: bool, total_len: usize) -> Vec<u8> {
    // 帧格式：[reportId, cmd, 填充]。发送前把整个载荷逐字节取反，
    // 全 0 尾部因此变成 0xFF。实机 L7 Pro 的 FeatureReportByteLength = 65。
    let mut req = vec![if inverted { 0xFF } else { 0x00 }; total_len];
    req[0] = rid;
    req[1] = CMD_DEVICE_STATUS ^ if inverted { 0xFF } else { 0 };
    req
}

/// 用一种「报告 ID + 变换 + 帧长」组合尝试「写命令 + 读回」。
fn try_read(
    dev: &HidDevice,
    rid: u8,
    inverted: bool,
    total_len: usize,
) -> Result<Reading, String> {
    let req = request_frame(rid, inverted, total_len);
    let mut last_raw = String::new();
    let inv = if inverted { 0xFF } else { 0 };

    for attempt in 0..8 {
        dev.send_feature_report(&req)
            .map_err(|e| format!("发送 0x{rid:02X}/0x06 失败：{e}"))?;
        thread::sleep(Duration::from_millis(25));

        let mut buf = vec![0u8; total_len];
        buf[0] = rid;
        let n = dev
            .get_feature_report(&mut buf)
            .map_err(|e| format!("读取 Feature Report 失败：{e}"))?;
        buf.truncate(n.max(2));
        last_raw = hex(&buf[..buf.len().min(24)]);

        // 设备不响应时会把刚写进去的帧原样回显，必须排除这种「假成功」。
        let echoed_request = buf.as_slice() == &req[..buf.len()];
        let echo_ok = buf.len() >= 2 && (buf[1] ^ inv) == CMD_DEVICE_STATUS;
        if echo_ok && !echoed_request && buf[2..].iter().any(|b| *b != 0) {
            let payload: Vec<u8> = buf[2..].iter().map(|b| b ^ inv).collect();
            if let Some((vid, pid, fw, flags, battery, charging)) = parse_payload(&payload) {
                // 刚唤醒的设备偶尔回一帧全零身份，vid==0 即为无效读数。
                if vid != 0 && battery <= 100 {
                    return Ok(Reading {
                        vid,
                        pid,
                        firmware: fw,
                        flags,
                        battery,
                        charging,
                        inverted,
                        frame_len: total_len,
                        label: last_raw,
                    });
                }
            }
        }
        thread::sleep(Duration::from_millis(25 + 30 * attempt));
    }
    Err(format!(
        "0x{rid:02X} 帧长 {total_len} 取反={inverted}：8 次重试后无有效响应（最后一帧 {last_raw}）"
    ))
}

/// 打开某个候选接口并读取电量。
///
/// 按接口所属协议分派：迈从走 Feature Report，狼蛛走中断报告。
pub fn read_battery(api: &HidApi, c: &Candidate) -> Result<Reading, String> {
    let dev = api
        .open_path(&c.path)
        .map_err(|e| format!("打开接口失败：{e}"))?;
    match c.protocol {
        Protocol::Feature => {
            dev.set_blocking_mode(true)
                .map_err(|e| format!("设置阻塞模式失败：{e}"))?;
            read_feature(&dev, c)
        }
        Protocol::AulaRaw => read_aula(&dev, c),
    }
}

/// 迈从：组合优先级 65 字节取反帧 → 21 字节取反帧 → 两种原样帧。
/// 只要有一个组合返回合法身份（vid != 0 且电量 0..=100）就立即返回。
fn read_feature(dev: &HidDevice, c: &Candidate) -> Result<Reading, String> {
    let combos = [
        (true, FEATURE_LEN_NATIVE),
        (true, FEATURE_LEN_COMPAT),
        (false, FEATURE_LEN_NATIVE),
        (false, FEATURE_LEN_COMPAT),
    ];
    let mut errors = Vec::new();
    for (inverted, len) in combos {
        match try_read(dev, REPORT_SHORT, inverted, len) {
            Ok(mut r) => {
                r.label = format!("{} | RAW {}", c.label(), r.label);
                return Ok(r);
            }
            Err(e) => errors.push(e),
        }
    }
    Err(errors.join("；"))
}

/// 解析狼蛛应答帧。帧头可能带 Report ID，所以两种起点都接受。
///
/// ```text
/// 20 01 00 64 … 85     pos=0，电量 = d[pos + 3]
/// 00 20 01 00 64 …     pos=1（hidapi 未剥掉 Report ID 时）
/// ```
pub fn parse_aula(d: &[u8]) -> Option<u8> {
    let pos = if d.len() >= 4 && d[0] == AULA_CMD[0] && d[1] == AULA_CMD[1] {
        0
    } else if d.len() >= 5 && d[1] == AULA_CMD[0] && d[2] == AULA_CMD[1] {
        1
    } else {
        return None;
    };
    let battery = *d.get(pos + 3)?;
    if battery > 100 {
        return None;
    }
    // 末字节是前面所有字节之和；帧长不足时跳过校验（不同接口长度不同）。
    if pos == 0 && d.len() >= AULA_REPORT_LEN {
        let sum = d[..d.len() - 1]
            .iter()
            .fold(0u8, |acc, b| acc.wrapping_add(*b));
        if sum != d[d.len() - 1] {
            return None;
        }
    }
    Some(battery)
}

/// 狼蛛：写 33 字节查询帧，再读中断输入报告。
fn read_aula(dev: &HidDevice, c: &Candidate) -> Result<Reading, String> {
    // [Report ID 0x00] [20 01] [00] 其余填充
    let mut frame = vec![0u8; AULA_REPORT_LEN];
    frame[1] = AULA_CMD[0];
    frame[2] = AULA_CMD[1];

    let mut last_raw = String::new();
    for attempt in 0..6 {
        // 丢掉可能积压的上报，避免把上一轮的应答当成本次结果。
        let mut junk = [0u8; 64];
        while dev.read_timeout(&mut junk, 10).unwrap_or(0) > 0 {}

        dev.write(&frame)
            .map_err(|e| format!("发送电量查询失败：{e}"))?;
        thread::sleep(Duration::from_millis(30));

        let mut buf = [0u8; AULA_REPORT_LEN];
        let n = dev.read_timeout(&mut buf, 600).unwrap_or(0);
        if n >= 4 {
            last_raw = hex(&buf[..n.min(16)]);
            if let Some(battery) = parse_aula(&buf[..n]) {
                return Ok(Reading {
                    vid: c.vid,
                    pid: c.pid,
                    firmware: 0,
                    flags: 0,
                    battery,
                    // 该协议只回电量，没有充电位。
                    charging: 0,
                    inverted: false,
                    frame_len: AULA_REPORT_LEN,
                    label: format!("{} | RAW {}", c.label(), last_raw),
                });
            }
        }
        thread::sleep(Duration::from_millis(40 + 40 * attempt as u64));
    }
    Err(format!("狼蛛查询 6 次后仍无有效应答（最后一帧：{last_raw}）"))
}

/// 诊断输出：列出候选接口，并对每个接口打印原始 Feature Report。
pub fn dump_candidates(extra: &[ExtraDevice]) -> String {
    let mut s = String::new();
    let api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => return format!("hidapi 初始化失败：{e}\n"),
    };
    let list = find_candidates(&api, extra);
    s.push_str(&format!("共找到 {} 个候选接口\n", list.len()));
    for c in &list {
        s.push_str(&format!(
            "\n- {}\n  path: {}\n  serial: {}\n",
            c.label(),
            c.path.to_string_lossy(),
            if c.serial.is_empty() { "-" } else { c.serial.as_str() }
        ));

        // 狼蛛只有中断报告，Feature Report 会直接报错，单独走一遍查询。
        if c.protocol == Protocol::AulaRaw {
            match api.open_path(&c.path) {
                Ok(dev) => {
                    let mut frame = vec![0u8; AULA_REPORT_LEN];
                    frame[1] = AULA_CMD[0];
                    frame[2] = AULA_CMD[1];
                    match dev.write(&frame) {
                        Ok(_) => {
                            thread::sleep(Duration::from_millis(40));
                            let mut rsp = [0u8; AULA_REPORT_LEN];
                            match dev.read_timeout(&mut rsp, 600) {
                                Ok(n) if n >= 4 => {
                                    s.push_str(&format!(
                                        "  查询帧：{}\n  应答：{}\n",
                                        hex(&frame[..4]),
                                        hex(&rsp[..n.min(20)])
                                    ));
                                    match parse_aula(&rsp[..n]) {
                                        Some(b) => s.push_str(&format!("  解析：电量={b}%\n")),
                                        None => s.push_str("  解析：帧头或校验不匹配\n"),
                                    }
                                }
                                Ok(_) => s.push_str("  无应答\n"),
                                Err(e) => s.push_str(&format!("  读取失败：{e}\n")),
                            }
                        }
                        Err(e) => s.push_str(&format!("  发送失败：{e}\n")),
                    }
                }
                Err(e) => s.push_str(&format!("  打开失败：{e}\n")),
            }
            continue;
        }

        match api.open_path(&c.path) {
            Ok(dev) => {
                for len in [FEATURE_LEN_NATIVE, FEATURE_LEN_COMPAT] {
                    let req = request_frame(REPORT_SHORT, true, len);
                    match dev.send_feature_report(&req) {
                        Ok(()) => {
                            thread::sleep(Duration::from_millis(40));
                            let mut rsp = vec![0u8; len];
                            rsp[0] = REPORT_SHORT;
                            match dev.get_feature_report(&mut rsp) {
                                Ok(n) => {
                                    let n = n.max(2).min(len);
                                    s.push_str(&format!("  {len}B 取反帧响应：{}\n", hex(&rsp[..n.min(24)])));
                                    let payload: Vec<u8> =
                                        rsp[2..n].iter().map(|b| b ^ 0xFF).collect();
                                    s.push_str(&format!("  {len}B 解码载荷：{}\n", hex(&payload[..payload.len().min(16)])));
                                    if let Some((vid, pid, fw, flags, b, ch)) = parse_payload(&payload) {
                                        s.push_str(&format!(
                                            "  {len}B 解析：vid={vid:04X} pid={pid:04X} fw={fw} flags={flags:02X} 电量={b}% 充电={ch}\n"
                                        ));
                                    }
                                }
                                Err(e) => s.push_str(&format!("  {len}B 读取失败：{e}\n")),
                            }
                        }
                        Err(e) => s.push_str(&format!("  {len}B 发送失败：{e}\n")),
                    }
                }
            }
            Err(e) => s.push_str(&format!("  打开失败：{e}\n")),
        }
    }
    s
}

use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
use regex::Regex;

#[tauri::command]
pub async fn probe_latency(host: String, port: u16, mode: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let clean_host = host.trim().to_string();
        if clean_host.is_empty() {
            return Ok("地址为空".to_string());
        }

        if mode == "tcp" {
            // 1:1 对标 ssh8.py 6196-6221 行: 真实 TCP 连接测速
            let target_port = if port == 0 { 22 } else { port };
            let socket_addrs = match (clean_host.as_str(), target_port).to_socket_addrs() {
                Ok(mut addrs) => match addrs.next() {
                    Some(a) => a,
                    None => return Ok("DNS 解析失败".to_string()),
                },
                Err(_) => return Ok("地址无效".to_string()),
            };

            let start = Instant::now();
            match TcpStream::connect_timeout(&socket_addrs, Duration::from_millis(2500)) {
                Ok(stream) => {
                    stream.set_nodelay(true).ok();
                    // 1:1 对标 ssh8.py: 尝试在 600ms 内预读取首个包或握手数据包，测得精确应用层 RTT
                    stream.set_read_timeout(Some(Duration::from_millis(600))).ok();
                    let mut buf = [0u8; 128];
                    let _ = std::io::Read::read(&mut &stream, &mut buf);

                    let elapsed = start.elapsed();
                    let ms = elapsed.as_secs_f64() * 1000.0;
                    Ok(format!("TCP {:.1} ms", ms))
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::TimedOut {
                        Ok(format!("TCP {} 超时", target_port))
                    } else if e.kind() == std::io::ErrorKind::ConnectionRefused {
                        Ok(format!("TCP {} 拒绝", target_port))
                    } else {
                        Ok(format!("TCP {} 失败", target_port))
                    }
                }
            }
        } else {
            // 1:1 对标 ssh8.py 6222-6239 行: ICMP (Ping)
            let is_win = cfg!(target_os = "windows");
            let mut cmd = std::process::Command::new("ping");

            if is_win {
                use std::os::windows::process::CommandExt;
                // CREATE_NO_WINDOW = 0x08000000 彻底阻止系统弹出 CMD 黑色黑框
                cmd.creation_flags(0x08000000);
                cmd.args(["-n", "1", "-w", "2500", &clean_host]);
            } else {
                cmd.args(["-c", "1", "-W", "3", &clean_host]);
            }

            match cmd.output() {
                Ok(output) => {
                    let raw_bytes = output.stdout;
                    // Windows 中文系统的 ping 输出是 GBK，这里按 UTF-8 做有损解码，
                    // 中文会变成替换字符，但 "时间=xx ms" 里的数字与 ms 是 ASCII，仍可匹配。
                    let lossy_str = String::from_utf8_lossy(&raw_bytes).to_string();

                    // 1:1 对标 ssh8.py: r'(?:时间|time)[=<\s]*([0-9]+(?:\.[0-9]+)?)\s*ms'
                    let re = Regex::new(r"(?i)(?:时间|time)[=<\s]*([0-9]+(?:\.[0-9]+)?)\s*ms").ok();
                    if let Some(r) = re {
                        if let Some(caps) = r.captures(&lossy_str) {
                            if let Some(m) = caps.get(1) {
                                if let Ok(val) = m.as_str().parse::<f64>() {
                                    return Ok(format!("ICMP {:.1} ms", val));
                                }
                            }
                        }
                    }

                    // 备用匹配: time<1ms 或 直接匹配 "XXms"
                    if lossy_str.contains("<1ms") || lossy_str.contains("< 1ms") || lossy_str.to_lowercase().contains("time<1") {
                        return Ok("ICMP <1 ms".to_string());
                    }

                    let re_simple = Regex::new(r"([0-9]+(?:\.[0-9]+)?)\s*ms").ok();
                    if let Some(r) = re_simple {
                        if let Some(caps) = r.captures(&lossy_str) {
                            if let Some(m) = caps.get(1) {
                                if let Ok(val) = m.as_str().parse::<f64>() {
                                    return Ok(format!("ICMP {:.1} ms", val));
                                }
                            }
                        }
                    }

                    if lossy_str.to_lowercase().contains("ttl=") {
                        return Ok("ICMP <1 ms".to_string());
                    }

                    Ok("ICMP 超时".to_string())
                }
                Err(_) => Ok("ICMP 探测失败".to_string()),
            }
        }
    })
    .await
    .map_err(|e| format!("测速执行错误: {}", e))?
}

/// 单次 Ping 的结构化结果（网络工具箱用）
#[derive(serde::Serialize)]
pub struct PingOutcome {
    pub ok: bool,
    /// -1 表示未测到
    pub rtt_ms: f64,
    /// ok / timeout / dns_fail / error
    pub status: String,
}

/// Windows 上 ping 输出是 GBK，这里按 UTF-8 有损解码后匹配数字（数字与 ms 都是 ASCII）
fn parse_ping_output(raw: &[u8]) -> PingOutcome {
    let text = String::from_utf8_lossy(raw).to_string();
    let lower = text.to_lowercase();

    if let Ok(re) = Regex::new(r"(?i)(?:时间|time)[=<\s]*([0-9]+(?:\.[0-9]+)?)\s*ms") {
        if let Some(caps) = re.captures(&text) {
            if let Some(m) = caps.get(1) {
                if let Ok(val) = m.as_str().parse::<f64>() {
                    return PingOutcome { ok: true, rtt_ms: val, status: "ok".into() };
                }
            }
        }
    }
    if lower.contains("<1ms") || lower.contains("< 1ms") || lower.contains("time<1") {
        return PingOutcome { ok: true, rtt_ms: 0.5, status: "ok".into() };
    }
    if let Ok(re) = Regex::new(r"([0-9]+(?:\.[0-9]+)?)\s*ms") {
        if let Some(caps) = re.captures(&text) {
            if let Some(m) = caps.get(1) {
                if let Ok(val) = m.as_str().parse::<f64>() {
                    return PingOutcome { ok: true, rtt_ms: val, status: "ok".into() };
                }
            }
        }
    }
    if lower.contains("ttl=") {
        return PingOutcome { ok: true, rtt_ms: 0.5, status: "ok".into() };
    }
    if lower.contains("找不到主机") || lower.contains("could not find host") || lower.contains("unknown host") {
        return PingOutcome { ok: false, rtt_ms: -1.0, status: "dns_fail".into() };
    }
    if lower.contains("超时") || lower.contains("timed out") || lower.contains("100% loss") {
        return PingOutcome { ok: false, rtt_ms: -1.0, status: "timeout".into() };
    }
    PingOutcome { ok: false, rtt_ms: -1.0, status: "error".into() }
}

/// 网络工具箱：对任意目标做一次 ICMP 探测，返回结构化结果
#[tauri::command]
pub async fn ping_once(host: String, timeout_ms: Option<u64>) -> Result<PingOutcome, String> {
    tokio::task::spawn_blocking(move || {
        let target = host.trim().to_string();
        if target.is_empty() {
            return PingOutcome { ok: false, rtt_ms: -1.0, status: "error".into() };
        }
        let timeout = timeout_ms.unwrap_or(2000).clamp(300, 10000).to_string();

        let is_win = cfg!(target_os = "windows");
        let mut cmd = std::process::Command::new("ping");
        if is_win {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW：不弹黑框
            cmd.args(["-n", "1", "-w", &timeout, &target]);
        } else {
            let secs = (timeout_ms.unwrap_or(2000) as f64 / 1000.0).ceil().to_string();
            cmd.args(["-c", "1", "-W", &secs, &target]);
        }

        match cmd.output() {
            Ok(output) => parse_ping_output(&output.stdout),
            Err(_) => PingOutcome { ok: false, rtt_ms: -1.0, status: "error".into() },
        }
    })
    .await
    .map_err(|e| format!("Ping 执行错误: {}", e))
}
/// 网络工具箱：对指定端口做一次 TCP 连接测速（带首个包预读，测得应用层 RTT）
#[tauri::command]
pub async fn tcp_ping_once(
    host: String,
    port: u16,
    timeout_ms: Option<u64>,
) -> Result<PingOutcome, String> {
    tokio::task::spawn_blocking(move || {
        let target = host.trim().to_string();
        if target.is_empty() || port == 0 {
            return PingOutcome { ok: false, rtt_ms: -1.0, status: "error".into() };
        }
        let timeout = timeout_ms.unwrap_or(2000).clamp(300, 10000);

        let addr = match (target.as_str(), port).to_socket_addrs() {
            Ok(mut it) => match it.next() {
                Some(a) => a,
                None => return PingOutcome { ok: false, rtt_ms: -1.0, status: "dns_fail".into() },
            },
            Err(_) => return PingOutcome { ok: false, rtt_ms: -1.0, status: "dns_fail".into() },
        };

        // 只计「TCP 握手」耗时。
        // 之前额外做了一次 read() 想拿 SSH banner 的耗时，但 HTTP/HTTPS 这类
        // 「客户端先说话」的服务不会主动发数据，read() 会一直等到 read_timeout
        // 才返回，于是局域网网关也会被算成 600ms+。TCP Ping 的语义就是握手 RTT。
        let start = Instant::now();
        match TcpStream::connect_timeout(&addr, Duration::from_millis(timeout)) {
            Ok(stream) => {
                stream.set_nodelay(true).ok();
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                PingOutcome { ok: true, rtt_ms: ms, status: "ok".into() }
            }
            Err(e) => {
                let status = if e.kind() == std::io::ErrorKind::TimedOut {
                    "timeout"
                } else if e.kind() == std::io::ErrorKind::ConnectionRefused {
                    "refused"
                } else {
                    "error"
                };
                PingOutcome { ok: false, rtt_ms: -1.0, status: status.into() }
            }
        }
    })
    .await
    .map_err(|e| format!("TCP Ping 执行错误: {}", e))
}
/// 端口扫描的单端口结果
#[derive(serde::Serialize)]
pub struct PortScanResult {
    pub port: u16,
    pub open: bool,
    pub rtt_ms: f64,
    /// open / closed / timeout / error
    pub status: String,
}

/// 网络工具箱：并发扫描一批端口
/// 端口数量由前端分批传入，这样前端可以显示进度并随时中止。
#[tauri::command]
pub async fn scan_ports(
    host: String,
    ports: Vec<u16>,
    timeout_ms: Option<u64>,
    concurrency: Option<usize>,
) -> Result<Vec<PortScanResult>, String> {
    tokio::task::spawn_blocking(move || {
        use std::sync::{Arc, Mutex};

        let target = host.trim().to_string();
        if target.is_empty() || ports.is_empty() {
            return Ok(Vec::new());
        }
        // 只解析一次地址，避免每个端口都做一次 DNS
        let base = match (target.as_str(), 0u16).to_socket_addrs() {
            Ok(mut it) => match it.next() {
                Some(a) => a,
                None => return Ok(Vec::new()),
            },
            Err(_) => return Ok(Vec::new()),
        };
        let timeout = Duration::from_millis(timeout_ms.unwrap_or(1000).clamp(100, 10000));
        let workers = concurrency.unwrap_or(64).clamp(1, 256);
        let chunk = ((ports.len() + workers - 1) / workers).max(1);

        let results: Arc<Mutex<Vec<PortScanResult>>> = Arc::new(Mutex::new(Vec::with_capacity(ports.len())));

        std::thread::scope(|scope| {
            for part in ports.chunks(chunk) {
                let sink = Arc::clone(&results);
                let ip = base.ip();
                scope.spawn(move || {
                    let mut local: Vec<PortScanResult> = Vec::with_capacity(part.len());
                    for &port in part {
                        let socket = std::net::SocketAddr::new(ip, port);
                        let start = Instant::now();
                        match TcpStream::connect_timeout(&socket, timeout) {
                            Ok(_) => {
                                let ms = start.elapsed().as_secs_f64() * 1000.0;
                                local.push(PortScanResult { port, open: true, rtt_ms: ms, status: "open".into() });
                            }
                            Err(e) => {
                                let status = if e.kind() == std::io::ErrorKind::TimedOut {
                                    "timeout"
                                } else if e.kind() == std::io::ErrorKind::ConnectionRefused {
                                    "closed"
                                } else {
                                    "error"
                                };
                                local.push(PortScanResult { port, open: false, rtt_ms: -1.0, status: status.into() });
                            }
                        }
                    }
                    if let Ok(mut guard) = sink.lock() {
                        guard.extend(local);
                    }
                });
            }
        });

        let mut out = match Arc::try_unwrap(results) {
            Ok(m) => m.into_inner().unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        out.sort_by_key(|r| r.port);
        Ok(out)
    })
    .await
    .map_err(|e| format!("端口扫描执行错误: {}", e))?
}
/// 子网扫描的单台设备结果
#[derive(serde::Serialize)]
pub struct SubnetHost {
    pub ip: String,
    pub alive: bool,
    pub rtt_ms: f64,
    /// 从系统 ARP 表读到的 MAC（拿不到为空）
    pub mac: String,
    /// 根据开放端口推断的设备类型提示
    pub device_hint: String,
}

/// 解析 "192.168.1.0/24" 或 "192.168.1.1-254"，返回可用主机地址列表
fn expand_cidr(cidr: &str) -> Vec<std::net::Ipv4Addr> {
    let expr = cidr.trim();
    if expr.is_empty() {
        return Vec::new();
    }
    // 形式一：a.b.c.d/prefix
    if let Some((ip_part, prefix_part)) = expr.split_once('/') {
        let base: std::net::Ipv4Addr = match ip_part.trim().parse() {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        let prefix: u32 = match prefix_part.trim().parse() {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        if prefix > 32 {
            return Vec::new();
        }
        // 扫描上限控制：前缀小于 /22（>1022 台）时按 /22 处理，避免误扫整个大网段
        let effective = prefix.max(22);
        let mask: u32 = if effective == 0 { 0 } else { u32::MAX << (32 - effective) };
        let base_u32 = u32::from(base);
        let network = base_u32 & mask;
        let host_bits = 32 - effective;
        let total = 1u32 << host_bits;
        let mut out = Vec::new();
        // 跳过网络地址与广播地址
        for i in 1..total.saturating_sub(1) {
            let addr = std::net::Ipv4Addr::from(network + i);
            out.push(addr);
        }
        return out;
    }
    // 形式二：a.b.c.start-end
    if let Some((head, tail)) = expr.split_once('-') {
        let head = head.trim();
        let tail = tail.trim();
        if let Some(dot) = head.rfind('.') {
            let prefix = &head[..dot];
            let start: u16 = head[dot + 1..].parse().unwrap_or(1);
            let end: u16 = tail.parse().unwrap_or(254);
            let (lo, hi) = if start <= end { (start, end) } else { (end, start) };
            let mut out = Vec::new();
            for last in lo.max(1)..=hi.min(254) {
                if let Ok(ip) = format!("{}.{}", prefix, last).parse::<std::net::Ipv4Addr>() {
                    out.push(ip);
                }
            }
            return out;
        }
    }
    Vec::new()
}

/// 读取系统 ARP 表，返回 ip -> mac 映射
fn read_arp_table() -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;
    let mut map = HashMap::new();
    let is_win = cfg!(target_os = "windows");
    let mut cmd = std::process::Command::new("arp");
    if is_win {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
        cmd.arg("-a");
    } else {
        cmd.arg("-an");
    }
    let output = match cmd.output() {
        Ok(o) => o,
        Err(_) => return map,
    };
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 2 {
            continue;
        }
        let maybe_ip = cols[0];
        let maybe_mac = cols[1];
        let looks_like_ip = maybe_ip.matches('.').count() == 3
            && maybe_ip.chars().all(|c| c.is_ascii_digit() || c == '.');
        let looks_like_mac = maybe_mac.matches('-').count() == 5 || maybe_mac.matches(':').count() == 5;
        if looks_like_ip && looks_like_mac {
            let mac = maybe_mac.replace('-', ":").to_uppercase();
            if mac != "FF:FF:FF:FF:FF:FF" {
                map.insert(maybe_ip.to_string(), mac);
            }
        }
    }
    map
}

/// 用开放端口粗略推断设备类型
fn guess_device_hint(ports: &[u16]) -> String {
    let has = |p: u16| ports.contains(&p);
    if has(9100) || has(515) || has(631) {
        return "打印机".into();
    }
    if has(554) || has(8000) && has(8080) {
        return "摄像头/录像机".into();
    }
    if has(3389) || has(445) || has(139) {
        return "Windows 设备".into();
    }
    if has(22) || has(23) {
        return "Linux/网络设备".into();
    }
    if has(80) || has(443) || has(8080) {
        return "Web 设备".into();
    }
    if !ports.is_empty() {
        return "有服务响应".into();
    }
    "在线设备".into()
}

/// 网络工具箱：扫描一个网段里在线的设备
#[tauri::command]
pub async fn scan_subnet(
    cidr: String,
    timeout_ms: Option<u64>,
    concurrency: Option<usize>,
) -> Result<Vec<SubnetHost>, String> {
    tokio::task::spawn_blocking(move || {
        use std::sync::{Arc, Mutex};

        let targets = expand_cidr(&cidr);
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let wait_ms = timeout_ms.unwrap_or(600).clamp(100, 5000);
        let workers = concurrency.unwrap_or(64).clamp(1, 256);
        let chunk = ((targets.len() + workers - 1) / workers).max(1);
        let is_win = cfg!(target_os = "windows");

        let alive: Arc<Mutex<Vec<(String, f64)>>> = Arc::new(Mutex::new(Vec::new()));

        std::thread::scope(|scope| {
            for part in targets.chunks(chunk) {
                let sink = Arc::clone(&alive);
                scope.spawn(move || {
                    let mut local: Vec<(String, f64)> = Vec::new();
                    for ip in part {
                        let ip_text = ip.to_string();
                        let start = Instant::now();
                        let mut cmd = std::process::Command::new("ping");
                        if is_win {
                            use std::os::windows::process::CommandExt;
                            cmd.creation_flags(0x08000000);
                            cmd.args(["-n", "1", "-w", &wait_ms.to_string(), &ip_text]);
                        } else {
                            let secs = ((wait_ms as f64) / 1000.0).ceil().max(1.0).to_string();
                            cmd.args(["-c", "1", "-W", &secs, &ip_text]);
                        }
                        if let Ok(out) = cmd.output() {
                            let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
                            let ok = text.contains("ttl=") || text.contains("时间=") || text.contains("time=");
                            if ok {
                                local.push((ip_text, start.elapsed().as_secs_f64() * 1000.0));
                            }
                        }
                    }
                    if let Ok(mut guard) = sink.lock() {
                        guard.extend(local);
                    }
                });
            }
        });

        let found = match Arc::try_unwrap(alive) {
            Ok(m) => m.into_inner().unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        let arp = read_arp_table();

        // 对在线设备再探一次常见端口，用于推断设备类型
        let mut results: Vec<SubnetHost> = Vec::with_capacity(found.len());
        for (ip_text, rtt) in found {
            let mac = arp.get(&ip_text).cloned().unwrap_or_default();
            let mut open_ports: Vec<u16> = Vec::new();
            if let Ok(parsed) = ip_text.parse::<std::net::Ipv4Addr>() {
                for port in [22u16, 23, 80, 139, 443, 445, 554, 3389, 9100] {
                    let socket = std::net::SocketAddr::new(std::net::IpAddr::V4(parsed), port);
                    if TcpStream::connect_timeout(&socket, Duration::from_millis(280)).is_ok() {
                        open_ports.push(port);
                    }
                }
            }
            results.push(SubnetHost {
                ip: ip_text,
                alive: true,
                rtt_ms: rtt,
                mac,
                device_hint: guess_device_hint(&open_ports),
            });
        }
        results.sort_by_key(|r| {
            r.ip.parse::<std::net::Ipv4Addr>()
                .map(|v| u32::from(v))
                .unwrap_or(0)
        });
        Ok(results)
    })
    .await
    .map_err(|e| format!("子网扫描执行错误: {}", e))?
}

/// 子网扫描（分批版）：前端把 IP 列表切块传进来，便于流式显示结果
#[tauri::command]
pub async fn scan_subnet_batch(
    ips: Vec<String>,
    timeout_ms: Option<u64>,
    concurrency: Option<usize>,
) -> Result<Vec<SubnetHost>, String> {
    tokio::task::spawn_blocking(move || {
        use std::sync::{Arc, Mutex};

        if ips.is_empty() {
            return Ok(Vec::new());
        }
        let wait_ms = timeout_ms.unwrap_or(600).clamp(100, 5000);
        let workers = concurrency.unwrap_or(32).clamp(1, 128);
        let chunk = ((ips.len() + workers - 1) / workers).max(1);
        let is_win = cfg!(target_os = "windows");

        let alive: Arc<Mutex<Vec<(String, f64)>>> = Arc::new(Mutex::new(Vec::new()));

        std::thread::scope(|scope| {
            for part in ips.chunks(chunk) {
                let sink = Arc::clone(&alive);
                scope.spawn(move || {
                    let mut local: Vec<(String, f64)> = Vec::new();
                    for ip_text in part {
                        let start = Instant::now();
                        let mut cmd = std::process::Command::new("ping");
                        if is_win {
                            use std::os::windows::process::CommandExt;
                            cmd.creation_flags(0x08000000);
                            cmd.args(["-n", "1", "-w", &wait_ms.to_string(), ip_text.as_str()]);
                        } else {
                            let secs = ((wait_ms as f64) / 1000.0).ceil().max(1.0).to_string();
                            cmd.args(["-c", "1", "-W", &secs, ip_text.as_str()]);
                        }
                        if let Ok(out) = cmd.output() {
                            let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
                            let ok = text.contains("ttl=") || text.contains("时间=") || text.contains("time=");
                            if ok {
                                local.push((ip_text.clone(), start.elapsed().as_secs_f64() * 1000.0));
                            }
                        }
                    }
                    if let Ok(mut guard) = sink.lock() {
                        guard.extend(local);
                    }
                });
            }
        });

        let found = match Arc::try_unwrap(alive) {
            Ok(m) => m.into_inner().unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        let arp = read_arp_table();

        let mut results: Vec<SubnetHost> = Vec::with_capacity(found.len());
        for (ip_text, rtt) in found {
            let mac = arp.get(&ip_text).cloned().unwrap_or_default();
            let mut open_ports: Vec<u16> = Vec::new();
            if let Ok(parsed) = ip_text.parse::<std::net::Ipv4Addr>() {
                for port in [22u16, 23, 80, 139, 443, 445, 554, 3389, 9100] {
                    let socket = std::net::SocketAddr::new(std::net::IpAddr::V4(parsed), port);
                    if TcpStream::connect_timeout(&socket, Duration::from_millis(280)).is_ok() {
                        open_ports.push(port);
                    }
                }
            }
            results.push(SubnetHost {
                ip: ip_text,
                alive: true,
                rtt_ms: rtt,
                mac,
                device_hint: guess_device_hint(&open_ports),
            });
        }
        Ok(results)
    })
    .await
    .map_err(|e| format!("子网扫描执行错误: {}", e))?
}
/// WOL 设备候选（探测用）
#[derive(serde::Serialize)]
pub struct WolCandidate {
    pub ip: String,
    pub mac: String,
    /// 当前是否 ping 得通
    pub alive: bool,
    /// 反向 DNS 解析到的主机名（拿不到为空）
    pub hostname: String,
    /// 提示：疑似待唤醒 / 在线
    pub hint: String,
}

/// Windows 下用 iphlpapi 的 GetIpAddrTable 直接读网卡表。
///
/// 为什么不再解析 ipconfig：中文系统的 ipconfig 输出是 GBK，「子网掩码」这个
/// 标签经 String::from_utf8_lossy 后变成乱码，`line.contains("掩码")` 永远不成立，
/// 解析结果恒为空 Vec —— 只剩全局广播可用，而 255.255.255.255 在本机有多条
/// 等价路由（VirtualBox / 物理网卡 metric 相同），会被系统送进虚拟网卡。
/// 直接调 API 既避开编码问题，也不用起子进程。
#[cfg(target_os = "windows")]
fn local_ipv4_networks_winapi() -> Vec<(std::net::Ipv4Addr, std::net::Ipv4Addr)> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct MibIpAddrRow {
        dw_addr: u32,
        dw_index: u32,
        dw_mask: u32,
        dw_bcast_addr: u32,
        dw_reasm_size: u32,
        unused1: u16,
        w_type: u16,
    }

    #[link(name = "iphlpapi")]
    unsafe extern "system" {
        fn GetIpAddrTable(table: *mut u8, size: *mut u32, order: i32) -> u32;
    }

    const NO_ERROR: u32 = 0;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    // 第一次调用只取所需缓冲区大小
    let mut size: u32 = 0;
    let first = unsafe { GetIpAddrTable(std::ptr::null_mut(), &mut size, 0) };
    if first != ERROR_INSUFFICIENT_BUFFER && first != NO_ERROR {
        return Vec::new();
    }
    if size < 4 {
        return Vec::new();
    }

    // 多要一点余量：两次调用之间网卡表可能变化
    let mut buf = vec![0u8; size as usize + 4096];
    let mut real_size = buf.len() as u32;
    let res = unsafe { GetIpAddrTable(buf.as_mut_ptr(), &mut real_size, 0) };
    if res != NO_ERROR {
        return Vec::new();
    }

    let num = unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const u32) } as usize;
    let row_size = std::mem::size_of::<MibIpAddrRow>();
    let mut out: Vec<(std::net::Ipv4Addr, std::net::Ipv4Addr)> = Vec::new();
    for i in 0..num {
        let off = 4 + i * row_size;
        if off + row_size > buf.len() {
            break;
        }
        let row: MibIpAddrRow =
            unsafe { std::ptr::read_unaligned(buf.as_ptr().add(off) as *const MibIpAddrRow) };
        // dwAddr / dwMask 是网络字节序
        let ip_raw = u32::from_be(row.dw_addr);
        let mask_raw = u32::from_be(row.dw_mask);
        // 跳过 /32（Tailscale 这类点对点网卡）和 /0
        if mask_raw == u32::MAX || mask_raw == 0 {
            continue;
        }
        // 跳过回环 127/8 和链路本地 169.254/16（网卡未连接时系统自动分配的 APIPA），
        // 这些网段发广播没有意义，只会把提示文字弄乱
        if (ip_raw >> 24) == 127 || (ip_raw & 0xFFFF_0000) == 0xA9FE_0000 {
            continue;
        }
        let ip = std::net::Ipv4Addr::from(ip_raw);
        let mask = std::net::Ipv4Addr::from(mask_raw);
        if !out.iter().any(|(a, b)| *a == ip && *b == mask) {
            out.push((ip, mask));
        }
    }
    out
}

/// 枚举本机所有 IPv4 网段（地址 + 掩码），用于逐个网段发送广播。
/// 多网卡场景（Tailscale / VirtualBox / 双网卡）下，只用 255.255.255.255
/// 可能被系统选错出口，魔术包发不到目标网段，所以必须按网段定向广播。
fn local_ipv4_networks() -> Vec<(std::net::Ipv4Addr, std::net::Ipv4Addr)> {
    #[cfg(target_os = "windows")]
    {
        let nets = local_ipv4_networks_winapi();
        if !nets.is_empty() {
            return nets;
        }
    }
    local_ipv4_networks_ipconfig()
}

/// 兜底路径：解析 ipconfig 文本（非 Windows 平台，或 API 调用异常时使用）。
fn local_ipv4_networks_ipconfig() -> Vec<(std::net::Ipv4Addr, std::net::Ipv4Addr)> {
    // 绝对路径调用：打包成 GUI 程序后进程 PATH 可能不含 System32，
    // 只写 "ipconfig" 会静默失败，导致网段枚举为空、只剩全局广播可用。
    let exe = std::env::var("SystemRoot")
        .map(|root| format!("{}\\System32\\ipconfig.exe", root))
        .unwrap_or_else(|_| "ipconfig".to_string());
    let mut cmd = std::process::Command::new(exe);
    if cfg!(target_os = "windows") {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let output = match cmd.output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    // 中文系统输出是 GBK，网卡名会乱码，但 IP / 掩码是 ASCII，不影响解析
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let re = match Regex::new(r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}") {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut result: Vec<(std::net::Ipv4Addr, std::net::Ipv4Addr)> = Vec::new();
    let mut pending: Option<std::net::Ipv4Addr> = None;
    for line in text.lines() {
        let ips: Vec<&str> = re.find_iter(line).map(|m| m.as_str()).collect();
        if ips.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        if lower.contains("ipv4") {
            if let Ok(ip) = ips[0].parse() {
                pending = Some(ip);
            }
        } else if lower.contains("subnet") || line.contains("掩码") {
            if let Some(ip) = pending.take() {
                if let Ok(mask) = ips[0].parse::<std::net::Ipv4Addr>() {
                    let mask_bits = u32::from(mask);
                    // 跳过 /32（Tailscale 这类点对点网卡发广播没有意义）
                    if mask_bits != u32::MAX && mask_bits != 0 {
                        result.push((ip, mask));
                    }
                }
            }
        }
    }
    result
}

/// 网络唤醒：发送 WOL 魔术包（6 个 0xFF + 16 次重复的 MAC）
#[tauri::command]
pub fn wake_on_lan(
    mac: String,
    broadcast: Option<String>,
    port: Option<u16>,
    target_ip: Option<String>,
) -> Result<String, String> {
    let clean: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_uppercase();
    if clean.len() != 12 {
        return Err("MAC 需要 12 位十六进制字符（如 AA:BB:CC:DD:EE:FF）".to_string());
    }
    let mut mac_bytes = [0u8; 6];
    for i in 0..6 {
        mac_bytes[i] = u8::from_str_radix(&clean[i * 2..i * 2 + 2], 16)
            .map_err(|_| "MAC 解析失败".to_string())?;
    }

    let mut packet: Vec<u8> = Vec::with_capacity(102);
    packet.extend_from_slice(&[0xFFu8; 6]);
    for _ in 0..16 {
        packet.extend_from_slice(&mac_bytes);
    }
    let target_port = port.unwrap_or(9);

    // 如果指定了广播地址，就先只发它（用户明确指定时以用户为准）
    // 注意：255.255.255.255 视为「自动」——它是旧版本的默认值，
    // 而全局广播在多数交换机/AP 上会被丢弃，必须改走按网段定向广播。
    if let Some(custom) = broadcast {
        let host = custom.trim().to_string();
        if !host.is_empty() && host != "255.255.255.255" {
            let addr = format!("{}:{}", host, target_port);
            let socket = std::net::UdpSocket::bind("0.0.0.0:0")
                .map_err(|e| format!("创建 socket 失败: {}", e))?;
            socket.set_broadcast(true).ok();
            socket
                .send_to(&packet, &addr)
                .map_err(|e| format!("发送到 {} 失败: {}", addr, e))?;
            return Ok(format!("已向 {} 发送 {} 字节魔术包", addr, packet.len()));
        }
    }

    let mut sent: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    // 单播模式：某些网卡只认指向自己的包，广播收不到。
    // 先用 arp -s 静态绑定 IP→MAC（否则目标离线时 ARP 解析失败，包根本发不出去），
    // 再往该 IP 直接单播一份。
    let last_target_ip: Option<String> = target_ip.clone();
    if let Some(ip_text) = target_ip {
        let ip_clean = ip_text.trim().to_string();
        if !ip_clean.is_empty() && ip_clean.parse::<std::net::Ipv4Addr>().is_ok() {
            let mac_dash = (0..6)
                .map(|i| format!("{:02X}", mac_bytes[i]))
                .collect::<Vec<_>>()
                .join("-");
            let mut arp_cmd = std::process::Command::new("arp");
            if cfg!(target_os = "windows") {
                use std::os::windows::process::CommandExt;
                arp_cmd.creation_flags(0x08000000);
            }
            arp_cmd.args(["-s", &ip_clean, &mac_dash]);
            let arp_ok = matches!(arp_cmd.output(), Ok(o) if o.status.success());
            if !arp_ok {
                notes.push("静态 ARP 写入失败（可能需要管理员权限）".to_string());
            }

            if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
                let port7 = 7u16;
                if socket
                    .send_to(&packet, (ip_clean.as_str(), target_port))
                    .is_ok()
                    || socket.send_to(&packet, (ip_clean.as_str(), port7)).is_ok()
                {
                    sent.push(format!("{}:{}（单播）", ip_clean, target_port));
                }
            }
        }
    }

    // 广播模式：对本机每个网段做定向广播，再补一次全局广播
    // 广播模式：完全照搬「验证可用」的方式——
    //   绑定 0.0.0.0，开启 SO_BROADCAST，发到每个网段的定向广播地址。
    // 之前绑定到具体网卡地址（bind((ip, 0))）虽然 send_to 返回成功，
    // 但实测包到不了目标网段；绑定 INADDR_ANY 才是可靠路径。
    let nets = local_ipv4_networks();
    let mut targets: Vec<std::net::Ipv4Addr> = Vec::new();
    for (ip, mask) in nets {
        let mask_bits = u32::from(mask);
        let bcast = std::net::Ipv4Addr::from(u32::from(ip) | !mask_bits);
        if !targets.contains(&bcast) {
            targets.push(bcast);
        }
    }
    // 兜底：网卡表枚举为空时（API 异常），用目标 IP 按 /24 推断广播地址。
    // 现在 target_ip 只是可选项了——WOL 唤醒本质上只需要 MAC，目标机的
    // DHCP 租期到期、IP 被回收、记录里根本没填 IP，都不该影响唤醒。
    if targets.is_empty() {
        if let Some(ip_text) = &last_target_ip {
            if let Ok(parsed) = ip_text.trim().parse::<std::net::Ipv4Addr>() {
                let v = u32::from(parsed);
                targets.push(std::net::Ipv4Addr::from(v | 0x0000_00FF));
            }
        }
    }

    // 关机态网卡才刚上电，头几个包经常丢，所以每个目标连发多轮。
    const ROUNDS: usize = 3;
    const ROUND_GAP: Duration = Duration::from_millis(30);

    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        socket.set_broadcast(true).ok();
        // WOL 标准端口 9 与备用端口 7 各发一份；
        // 不依赖设备记录里的 port 字段（那个字段保留兼容，但发送固定用 9/7）
        for addr in &targets {
            let mut ok = false;
            for round in 0..ROUNDS {
                for port in [9u16, 7u16] {
                    if socket.send_to(&packet, (*addr, port)).is_ok() {
                        ok = true;
                    }
                }
                if round + 1 < ROUNDS {
                    std::thread::sleep(ROUND_GAP);
                }
            }
            if ok {
                sent.push(addr.to_string());
            }
        }

        // 全局广播放最后：本机只要有虚拟网卡（VirtualBox / VMware），
        // 255.255.255.255 就存在多条 metric 相同的等价路由，Windows 可能
        // 把包送进虚拟网卡而不是物理网卡。所以它只作为最后的补充，
        // 真正起作用的是上面按网段算出来的定向广播。
        let global = std::net::Ipv4Addr::new(255, 255, 255, 255);
        if !targets.contains(&global) {
            let mut ok = false;
            for port in [9u16, 7u16] {
                if socket.send_to(&packet, (global, port)).is_ok() {
                    ok = true;
                }
            }
            if ok {
                sent.push(global.to_string());
            }
        }
    }

    if sent.is_empty() {
        return Err("没有可用的发送出口（检查网络连接）".to_string());
    }
    let mut msg = format!("魔术包已发往 {}（端口 9 和 7 各一份）", sent.join(" / "));
    if !notes.is_empty() {
        msg.push_str(&format!("；注意：{}", notes.join("；")));
    }
    Ok(msg)
}

/// 反向 DNS 查主机名。内网里能不能拿到取决于路由器的 DNS 配置
/// （OpenWrt / 爱快等常会把 DHCP 租约注册成 A 记录，就能反查回来）。
fn reverse_dns(ip: &str) -> String {
    let is_win = cfg!(target_os = "windows");
    let mut cmd = std::process::Command::new("nslookup");
    if is_win {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
        cmd.args(["-timeout=1", "-retry=1", ip]);
    } else {
        cmd.args(["-timeout=1", "-retries=1", ip]);
    }
    let output = match cmd.output() {
        Ok(o) => o,
        Err(_) => return String::new(),
    };
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    for line in text.lines() {
        let trimmed = line.trim();
        // 英文系统 Name: xxx，中文系统 名称: xxx
        if let Some(rest) = trimmed.strip_prefix("Name:") {
            return rest.trim().to_string();
        }
        if let Some(rest) = trimmed.strip_prefix("名称:") {
            return rest.trim().to_string();
        }
    }
    String::new()
}

/// 探测局域网里可能支持 WOL 的设备。
/// 原理：关机后网卡仍在待机，ARP 表里仍有记录但 ping 不通 —— 这类最可能是可唤醒设备。
#[tauri::command]
pub async fn detect_wol_candidates(
    cidr: String,
    timeout_ms: Option<u64>,
) -> Result<Vec<WolCandidate>, String> {
    tokio::task::spawn_blocking(move || {
        use std::sync::{Arc, Mutex};

        let targets = expand_cidr(&cidr);
        let wait_ms = timeout_ms.unwrap_or(500).clamp(100, 3000);
        let is_win = cfg!(target_os = "windows");

        let ip_set: std::collections::HashSet<String> =
            targets.iter().map(|ip| ip.to_string()).collect();

        let alive: Arc<Mutex<std::collections::HashSet<String>>> =
            Arc::new(Mutex::new(std::collections::HashSet::new()));

        if !targets.is_empty() {
            let workers = 64usize.min(targets.len().max(1));
            let chunk = ((targets.len() + workers - 1) / workers).max(1);
            std::thread::scope(|scope| {
                for part in targets.chunks(chunk) {
                    let sink = Arc::clone(&alive);
                    scope.spawn(move || {
                        for ip in part {
                            let ip_text = ip.to_string();
                            let mut cmd = std::process::Command::new("ping");
                            if is_win {
                                use std::os::windows::process::CommandExt;
                                cmd.creation_flags(0x08000000);
                                cmd.args(["-n", "1", "-w", &wait_ms.to_string(), &ip_text]);
                            } else {
                                let secs = ((wait_ms as f64) / 1000.0).ceil().max(1.0).to_string();
                                cmd.args(["-c", "1", "-W", &secs, &ip_text]);
                            }
                            if let Ok(out) = cmd.output() {
                                let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
                                let ok = text.contains("ttl=")
                                    || text.contains("时间=")
                                    || text.contains("time=");
                                if ok {
                                    if let Ok(mut guard) = sink.lock() {
                                        guard.insert(ip_text);
                                    }
                                }
                            }
                        }
                    });
                }
            });
        }

        let alive_set = match Arc::try_unwrap(alive) {
            Ok(m) => m.into_inner().unwrap_or_default(),
            Err(_) => std::collections::HashSet::new(),
        };

        // 关键：ARP 表要在 ping 之后再读。
        // ping 一个 IP 时系统必须先做 ARP 解析，所以「关机但网卡待机」的设备
        // 虽然 ping 不通，但这一步会把它的 MAC 写进 ARP 表——这正是可唤醒的信号。
        // 扫描前就读表的话，从来没通信过的设备根本不在表里，自然探测不到。
        let arp = read_arp_table();

        let mut out: Vec<WolCandidate> = Vec::new();
        for (ip, mac) in arp.iter() {
            // 只保留本网段内的记录，避免把无关的历史条目也列出来
            if !ip_set.is_empty() && !ip_set.contains(ip) {
                continue;
            }
            let alive = alive_set.contains(ip);
            out.push(WolCandidate {
                ip: ip.clone(),
                mac: mac.clone(),
                alive,
                hostname: reverse_dns(ip),
                hint: if alive { "在线".to_string() } else { "疑似待唤醒".to_string() },
            });
        }
        // 疑似待唤醒的排前面，方便直接添加
        out.sort_by(|a, b| {
            a.alive
                .cmp(&b.alive)
                .then_with(|| a.ip.cmp(&b.ip))
        });
        Ok(out)
    })
    .await
    .map_err(|e| format!("WOL 探测错误: {}", e))?
}
/// 指定单个 IP 探测（比整段扫描更精准，适合知道 IP 但不记得 MAC 的情况）
#[tauri::command]
pub async fn probe_wol_ip(ip: String) -> Result<WolCandidate, String> {
    tokio::task::spawn_blocking(move || {
        let target = ip.trim().to_string();
        if target.is_empty() {
            return Err("IP 为空".to_string());
        }
        let is_win = cfg!(target_os = "windows");
        // 先 ping 一次：这一步会触发系统的 ARP 解析，
        // 「关机但网卡待机」的设备虽然不回 ICMP，但会回 ARP，于是会出现在 ARP 表里
        let mut cmd = std::process::Command::new("ping");
        if is_win {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
            cmd.args(["-n", "1", "-w", "600", target.as_str()]);
        } else {
            cmd.args(["-c", "1", "-W", "1", target.as_str()]);
        }
        let alive = match cmd.output() {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
                text.contains("ttl=") || text.contains("时间=") || text.contains("time=")
            }
            Err(_) => false,
        };

        let arp = read_arp_table();
        let mac = arp.get(&target).cloned().unwrap_or_default();
        let hostname = reverse_dns(&target);

        Ok(WolCandidate {
            ip: target,
            mac,
            alive,
            hostname,
            hint: if alive {
                "在线".to_string()
            } else if !arp.is_empty() {
                "疑似待唤醒".to_string()
            } else {
                "未响应".to_string()
            },
        })
    })
    .await
    .map_err(|e| format!("探测错误: {}", e))?
}

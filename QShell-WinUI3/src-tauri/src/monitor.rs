use serde::{Deserialize, Serialize};
use std::io::Read;
use std::net::ToSocketAddrs;
use std::sync::Mutex;
use std::collections::HashMap;
use crate::ssh::SshManager;

#[derive(Debug, Clone, Default)]
struct SessionNetSnapshot {
    sample_ms: u64,
    rx_bytes: u64,
    tx_bytes: u64,
    if_name: String,
    smoothed_rx_bps: f64,
    smoothed_tx_bps: f64,
}

static SESSION_NET_CACHE: Mutex<Option<HashMap<String, SessionNetSnapshot>>> = Mutex::new(None);

#[derive(Default)]
struct SessionMetricCache {
    prev_cpu_idle: u64,
    prev_cpu_total: u64,
    last_cpu: u32,
    last_mem: u32,
    last_disk: u32,
}

static SESSION_METRIC_CACHE: Mutex<Option<HashMap<String, SessionMetricCache>>> = Mutex::new(None);

/// 会话关闭时清理该会话的监控缓存，否则每次重连都会在 HashMap 里泄漏一条记录。
pub(crate) fn clear_session_caches(session_id: &str) {
    if let Ok(mut cache) = SESSION_NET_CACHE.lock() {
        if let Some(map) = cache.as_mut() {
            map.remove(session_id);
        }
    }
    if let Ok(mut cache) = SESSION_METRIC_CACHE.lock() {
        if let Some(map) = cache.as_mut() {
            map.remove(session_id);
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RemoteSystemMetrics {
    pub cpu: u32,
    pub mem: u32,
    pub disk: u32,
    pub net_up: String,
    pub net_down: String,
    pub net_if: Option<String>,
    pub net_if_options: Vec<String>,
    pub cpu_model: Option<String>,
    pub cpu_cores: u32,
    pub mem_total_bytes: u64,
    pub mem_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub disk_used_bytes: u64,
}

#[tauri::command]
pub async fn fetch_system_metrics(
    state: tauri::State<'_, SshManager>,
    session_id: String,
    interface: Option<String>,
) -> Result<RemoteSystemMetrics, String> {
    let (monitor_slot, params) = {
        let guard = state.sessions.lock();
        let act = guard.get(&session_id).ok_or("SSH 会话不存在")?;
        (act.monitor.clone(), act.params.clone())
    };
    let sess_id_clone = session_id.clone();

    tokio::task::spawn_blocking(move || {
        let handle = crate::sftp::ensure_sftp_handle(&monitor_slot, &params)?;
        let session_arc = handle.session.clone();
        let cmd = "cat /etc/openwrt_release 2>/dev/null; grep '^cpu ' /proc/stat 2>/dev/null; cat /proc/meminfo 2>/dev/null || LANG=C free -m 2>/dev/null; df -m / 2>/dev/null | awk 'NR>1 {for(i=1;i<=NF;i++) if($i ~ /%$/) print $i}' || df -m 2>/dev/null; echo \"Cores: $(nproc 2>/dev/null || grep -c '^processor' /proc/cpuinfo 2>/dev/null)\"; grep -m1 -E 'model name|Hardware|Processor' /proc/cpuinfo 2>/dev/null; echo \"DF: $(df -Pk / 2>/dev/null | awk 'NR==2 {print $2, $3, $4}')\"; ip route show default 2>/dev/null; cat /proc/net/dev 2>/dev/null";
        
        let mut output = String::new();
        let start_time = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(1500);

        // 尝试从当前已连接的主 session 打开执行通道
        let mut executed = false;
        let mut attempts = 0;
        while attempts < 10 && start_time.elapsed() < timeout {
            attempts += 1;
            // libssh2 的 Session 不是线程安全的：取通道、执行和读取必须在同一把锁内完成，
            // 否则并发的 fetch_system_metrics 会同时操作同一个 session。
            let sess = session_arc.lock();
            let mut ch = match sess.channel_session() {
                Ok(ch) => ch,
                Err(_) => {
                    drop(sess);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    continue;
                }
            };
            let _ = ch.setenv("LANG", "C");
            if ch.exec(cmd).is_err() {
                let _ = ch.wait_close();
                drop(ch);
                drop(sess);
                std::thread::sleep(std::time::Duration::from_millis(20));
                continue;
            }
            let mut buf = [0u8; 4096];
            let read_start = std::time::Instant::now();
            loop {
                match ch.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        output.push_str(&String::from_utf8_lossy(&buf[..n]));
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if read_start.elapsed() > std::time::Duration::from_millis(600) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
            let _ = ch.wait_close();
            drop(ch);
            drop(sess);
            executed = true;
            break;
        }

        // 如果主会话因 Shell 独占阻塞，回退启动独立轻量轻击线程连接（1:1 对标 paramiko client 行为）
        if !executed || output.trim().is_empty() {
            // 不能用 "host:port".parse::<SocketAddr>()：那只接受 IP 字面量，
            // 域名/IPv6 会解析失败并静默回退到 127.0.0.1:22（等于用远端凭据认证本机）。
            let aux_tcp = (params.host.as_str(), params.port)
                .to_socket_addrs()
                .map(|addrs| {
                    addrs
                        .filter_map(|addr| {
                            std::net::TcpStream::connect_timeout(
                                &addr,
                                std::time::Duration::from_millis(800),
                            )
                            .ok()
                        })
                        .next()
                })
                .unwrap_or(None);
            if let Some(tcp) = aux_tcp {
                let _ = tcp.set_nodelay(true);
                if let Ok(mut aux_sess) = ssh2::Session::new() {
                    aux_sess.set_tcp_stream(tcp);
                    // 建连超时只覆盖 TCP；不设会话超时会让握手/读取永久阻塞，
                    // 配合前端每 3 秒一次的轮询会逐步耗尽 blocking 线程池。
                    aux_sess.set_timeout(8_000);
                    if aux_sess.handshake().is_ok() {
                        if let Some(ref key_path) = params.private_key {
                            if !key_path.trim().is_empty() {
                                let p = std::path::Path::new(key_path);
                                if p.is_file() {
                                    let _ = aux_sess.userauth_pubkey_file(&params.username, None, p, params.passphrase.as_deref());
                                }
                            }
                        }
                        if !aux_sess.authenticated() {
                            if let Some(ref pwd) = params.password {
                                let _ = aux_sess.userauth_password(&params.username, pwd);
                            }
                        }
                        if aux_sess.authenticated() {
                            if let Ok(mut ch) = aux_sess.channel_session() {
                                if ch.exec(cmd).is_ok() {
                                    let mut buf = [0u8; 4096];
                                    while let Ok(n) = ch.read(&mut buf) {
                                        if n == 0 { break; }
                                        output.push_str(&String::from_utf8_lossy(&buf[..n]));
                                    }
                                    let _ = ch.wait_close();
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut metric_caches = SESSION_METRIC_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let cache = metric_caches
            .get_or_insert_with(HashMap::new)
            .entry(sess_id_clone.clone())
            .or_default();

        let mut cpu = cache.last_cpu;
        let mut mem = 0;
        let mut disk = 0;

        let mut mem_kbs: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        let mut rx_bytes_total: u64 = 0;
        let mut tx_bytes_total: u64 = 0;
        let mut detected_if_name: Option<String> = None;

        let mut is_openwrt = false;
        let mut if_stats_map: Vec<(String, u64, u64)> = Vec::new();
        let mut default_if_name = String::new();
        let mut cpu_model: Option<String> = None;
        let mut cpu_cores = 0u32;
        let mut mem_total_bytes = 0u64;
        let mut mem_used_bytes = 0u64;
        let mut disk_total_bytes = 0u64;
        let mut disk_used_bytes = 0u64;

        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Cores:") {
                cpu_cores = trimmed
                    .split_once(':')
                    .and_then(|(_, v)| v.trim().parse().ok())
                    .unwrap_or(0);
            } else if trimmed.starts_with("DF:") {
                let values: Vec<u64> = trimmed[3..]
                    .split_whitespace()
                    .filter_map(|v| v.parse().ok())
                    .collect();
                if values.len() >= 2 {
                    disk_total_bytes = values[0].saturating_mul(1024);
                    disk_used_bytes = values[1].saturating_mul(1024);
                    if disk_total_bytes > 0 && disk == 0 {
                        disk = max_bounded(((disk_used_bytes as f64 / disk_total_bytes as f64) * 100.0).round() as u32);
                    }
                }
            } else if trimmed.contains("model name")
                || trimmed.starts_with("Hardware")
                || trimmed.starts_with("Processor")
            {
                if let Some((_, v)) = trimmed.split_once(':') {
                    let value = v.trim();
                    if !value.is_empty() {
                        cpu_model = Some(value.to_string());
                    }
                }
            } else if trimmed.starts_with("default ") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if let Some(pos) = parts.iter().position(|p| *p == "dev") {
                    if let Some(name) = parts.get(pos + 1) {
                        default_if_name = name.to_string();
                    }
                }
            } else if trimmed.contains("DISTRIB_ID") && (trimmed.contains("OpenWrt") || trimmed.contains("LEDE") || trimmed.contains("ImmortalWrt")) {
                is_openwrt = true;
            } else if trimmed.contains("OPENWRT") || trimmed.contains("openwrt") {
                is_openwrt = true;
            } else if trimmed.starts_with("cpu ") {
                let parts: Vec<&str> = trimmed.split_whitespace().skip(1).collect();
                if parts.len() >= 4 {
                    let fields: Vec<f64> = parts.iter().filter_map(|s| s.parse().ok()).collect();
                    if fields.len() >= 4 {
                        let idle = fields[3] + if fields.len() > 4 { fields[4] } else { 0.0 };
                        // guest/guest_nice 已分别包含在 user/nice 中，重复相加会让 total 虚大、CPU 偏低。
                        let total: f64 = fields[..fields.len().min(8)].iter().sum();
                        let p_idle = std::mem::replace(&mut cache.prev_cpu_idle, idle as u64) as f64;
                        let p_total = std::mem::replace(&mut cache.prev_cpu_total, total as u64) as f64;
                        if p_total > 0.0 && total > p_total {
                            let d_total = total - p_total;
                            let d_idle = idle - p_idle;
                            let calc_cpu = max_bounded(((1.0 - (d_idle / d_total)) * 100.0).round() as u32);
                            cpu = calc_cpu;
                            cache.last_cpu = cpu;
                        }
                    }
                }
            } else if trimmed.contains(':') && (
                trimmed.starts_with("MemTotal:") || trimmed.starts_with("MemFree:") ||
                trimmed.starts_with("MemAvailable:") || trimmed.starts_with("Buffers:") ||
                trimmed.starts_with("Cached:") || trimmed.starts_with("SReclaimable:") ||
                trimmed.starts_with("Shmem:")
            ) {
                // 1:1 对标 ssh8.py 第 9779-9786 行
                if let Some((k, rest)) = trimmed.split_once(':') {
                    if let Some(v_str) = rest.split_whitespace().next() {
                        if let Ok(val) = v_str.parse::<f64>() {
                            mem_kbs.insert(k.trim().to_string(), val);
                        }
                    }
                }
            } else if trimmed.starts_with("Mem:") {
                // 1:1 对标 ssh8.py 第 9787-9796 行: 回退兼容 free -m 输出
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 3 {
                    if let (Ok(t_val), Ok(u_val)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                        if t_val > 0.0 {
                            mem = max_bounded(((u_val / t_val) * 100.0).round() as u32);
                        }
                    }
                }
            } else if trimmed.ends_with('%') && !trimmed.starts_with("cpu") && !trimmed.starts_with("Mem") {
                // 1:1 对标 ssh8.py 第 9797-9801 行
                let token = trimmed.trim_end_matches('%').trim();
                if let Ok(d) = token.parse::<u32>() {
                    disk = max_bounded(d);
                }
            } else if trimmed.contains(':') && !trimmed.starts_with("Mem") {
                // /proc/net/dev 流量提取
                let parts: Vec<&str> = trimmed.split(':').collect();
                if parts.len() == 2 {
                    let if_name = parts[0].trim();
                    if if_name != "lo" {
                        let nums: Vec<u64> = parts[1]
                            .split_whitespace()
                            .filter_map(|s| s.parse().ok())
                            .collect();
                        if nums.len() >= 9 {
                            let rx = nums[0];
                            let tx = nums[8];
                            if_stats_map.push((if_name.to_string(), rx, tx));
                        }
                    }
                }
            }
        }

        // 用户手动选择的网卡优先；未选择时才走自动策略。
        let mut net_if_options: Vec<String> = if_stats_map
            .iter()
            .map(|(name, _, _)| name.clone())
            .collect();
        net_if_options.sort_by_key(|name| name.to_lowercase());

        if let Some(requested) = interface.as_deref() {
            if let Some((name, rx, tx)) = if_stats_map
                .iter()
                .find(|(name, _, _)| name == requested)
            {
                detected_if_name = Some(name.clone());
                rx_bytes_total = *rx;
                tx_bytes_total = *tx;
            }
        }

        // 网卡适配策略：
        // 1. 如果检测到是 OpenWrt 系 (或包含 pppoe/br-lan 网卡特征)：
        //    - 优先匹配 pppoe 相关网卡 (如 pppoe-wan, pppoe, ppp0 等)
        //    - 若无，回退匹配 br-lan 相关网卡 (如 br-lan, br0 等)
        // 2. 若仍未匹配，或非 OpenWrt 系：检测其他网卡
        //    - 排除虚拟干扰网卡 (docker, veth, ifb, dummy, tun, tap, wg)
        //    - 优先选流量最大的物理网卡 (eth*, ens*, enp*, eno*, wlan*)
        //    - 最后兜底全局最大流量网卡
        if detected_if_name.is_none() && is_openwrt {
            // 优先级 A: pppoe 相关
            if let Some((name, rx, tx)) = if_stats_map.iter().find(|(name, _, _)| {
                let n = name.to_lowercase();
                n.starts_with("pppoe") || n.starts_with("ppp")
            }) {
                detected_if_name = Some(name.clone());
                rx_bytes_total = *rx;
                tx_bytes_total = *tx;
            }
            // 优先级 B: 回退到 br-lan 相关
            else if let Some((name, rx, tx)) = if_stats_map.iter().find(|(name, _, _)| {
                let n = name.to_lowercase();
                n.starts_with("br-lan") || n == "br0"
            }) {
                detected_if_name = Some(name.clone());
                rx_bytes_total = *rx;
                tx_bytes_total = *tx;
            }
        }

        // 非 OpenWrt：优先使用默认路由网卡
        if detected_if_name.is_none() && !default_if_name.is_empty() {
            if let Some((name, rx, tx)) = if_stats_map
                .iter()
                .find(|(name, _, _)| name == &default_if_name)
            {
                detected_if_name = Some(name.clone());
                rx_bytes_total = *rx;
                tx_bytes_total = *tx;
            }
        }

        // 若仍未确定，由通用智能策略挑选最佳网卡
        if detected_if_name.is_none() {
            // 候选 1：物理/常见上行网卡 (eth, ens, enp, eno, wan, wlan)，排除虚拟环回与容器网卡
            let physical_candidates: Vec<&(String, u64, u64)> = if_stats_map.iter().filter(|(name, _, _)| {
                let n = name.to_lowercase();
                !n.starts_with("docker") && !n.starts_with("veth") && !n.starts_with("ifb") &&
                !n.starts_with("dummy") && !n.starts_with("flannel") && !n.starts_with("cni")
            }).collect();

            // 在物理候选网卡中挑选当前累计总流量 (rx + tx) 最大的接口
            if let Some((name, rx, tx)) = physical_candidates.into_iter().max_by_key(|(_, rx, tx)| rx + tx) {
                detected_if_name = Some(name.clone());
                rx_bytes_total = *rx;
                tx_bytes_total = *tx;
            } else if let Some((name, rx, tx)) = if_stats_map.iter().max_by_key(|(_, rx, tx)| rx + tx) {
                // 最终任意网卡兜底
                detected_if_name = Some(name.clone());
                rx_bytes_total = *rx;
                tx_bytes_total = *tx;
            }
        }

        if let Some(&total_kb) = mem_kbs.get("MemTotal") {
            if total_kb > 0.0 {
                let avail_kb = if let Some(&available) = mem_kbs.get("MemAvailable") {
                    available
                } else {
                    let free_kb = *mem_kbs.get("MemFree").unwrap_or(&0.0);
                    let buf_kb = *mem_kbs.get("Buffers").unwrap_or(&0.0);
                    let cache_kb = *mem_kbs.get("Cached").unwrap_or(&0.0);
                    let srec_kb = *mem_kbs.get("SReclaimable").unwrap_or(&0.0);
                    let shmem_kb = *mem_kbs.get("Shmem").unwrap_or(&0.0);
                    (free_kb + buf_kb + cache_kb + srec_kb - shmem_kb)
                        .max(0.0)
                        .min(total_kb)
                };
                mem_total_bytes = (total_kb * 1024.0) as u64;
                mem_used_bytes = ((total_kb - avail_kb).max(0.0) * 1024.0) as u64;
            }
        }

        // 阶梯式全系统内存占用精确计算 (1:1 对标 ssh8.py 第 9833-9852 行)
        if mem == 0 {
            if let Some(&total_kb) = mem_kbs.get("MemTotal") {
                if total_kb > 0.0 {
                    if let Some(&avail_kb) = mem_kbs.get("MemAvailable") {
                        let used_kb = (total_kb - avail_kb).max(0.0);
                        mem = max_bounded(((used_kb / total_kb) * 100.0).round() as u32);
                    } else {
                        let free_kb = *mem_kbs.get("MemFree").unwrap_or(&0.0);
                        let buf_kb = *mem_kbs.get("Buffers").unwrap_or(&0.0);
                        let cache_kb = *mem_kbs.get("Cached").unwrap_or(&0.0);
                        let srec_kb = *mem_kbs.get("SReclaimable").unwrap_or(&0.0);
                        let shmem_kb = *mem_kbs.get("Shmem").unwrap_or(&0.0);
                        let mut avail_kb = free_kb + buf_kb + cache_kb + srec_kb - shmem_kb;
                        avail_kb = avail_kb.max(0.0).min(total_kb);
                        let used_kb = (total_kb - avail_kb).max(0.0);
                        mem = max_bounded(((used_kb / total_kb) * 100.0).round() as u32);
                    }
                }
            }
        }

        // 保留快照，避免单次采样瞬间为 0 时归零断闪
        if mem > 0 {
            cache.last_mem = mem;
        } else {
            mem = cache.last_mem;
        }

        if disk > 0 {
            cache.last_disk = disk;
        } else {
            disk = cache.last_disk;
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let current_if_name = detected_if_name.clone().unwrap_or_default();

        let mut speed_tx = 0.0;
        let mut speed_rx = 0.0;

        if let Ok(mut cache_guard) = SESSION_NET_CACHE.lock() {
            let map = cache_guard.get_or_insert_with(HashMap::new);
            if let Some(prev) = map.get_mut(&sess_id_clone) {
                // 如果网卡发生改变、或时间间隔异常 (<500ms 或 >15s)、或字节计数器回退(重启/溢出)
                let dt = (now_ms.saturating_sub(prev.sample_ms)) as f64 / 1000.0;
                let is_same_if = prev.if_name == current_if_name;
                let is_valid_time = dt >= 0.5 && dt <= 15.0;
                let is_valid_bytes = rx_bytes_total >= prev.rx_bytes && tx_bytes_total >= prev.tx_bytes;

                if is_same_if && is_valid_time && is_valid_bytes {
                    let raw_rx_bps = (rx_bytes_total - prev.rx_bytes) as f64 / dt;
                    let raw_tx_bps = (tx_bytes_total - prev.tx_bytes) as f64 / dt;

                    // 异常突刺过滤：单卡速度上限限制为 100Gbps (12.5GB/s)，超过判定为计数器跳变异常，予以平滑兜底
                    const MAX_REASONABLE_BPS: f64 = 12.5 * 1024.0 * 1024.0 * 1024.0;
                    let safe_rx_bps = if raw_rx_bps < MAX_REASONABLE_BPS { raw_rx_bps } else { prev.smoothed_rx_bps };
                    let safe_tx_bps = if raw_tx_bps < MAX_REASONABLE_BPS { raw_tx_bps } else { prev.smoothed_tx_bps };

                    // 指数加权移动平均 (EWMA) 平滑处理：本次测量 65% + 历史滑动值 35%
                    speed_rx = if prev.smoothed_rx_bps > 0.0 {
                        safe_rx_bps * 0.65 + prev.smoothed_rx_bps * 0.35
                    } else {
                        safe_rx_bps
                    };

                    speed_tx = if prev.smoothed_tx_bps > 0.0 {
                        safe_tx_bps * 0.65 + prev.smoothed_tx_bps * 0.35
                    } else {
                        safe_tx_bps
                    };

                    prev.smoothed_rx_bps = speed_rx;
                    prev.smoothed_tx_bps = speed_tx;
                } else {
                    // 网卡切换或初次重新同步时，重置滑动平均，不输出虚假峰值
                    prev.smoothed_rx_bps = 0.0;
                    prev.smoothed_tx_bps = 0.0;
                }

                prev.sample_ms = now_ms;
                prev.rx_bytes = rx_bytes_total;
                prev.tx_bytes = tx_bytes_total;
                prev.if_name = current_if_name;
            } else {
                map.insert(sess_id_clone, SessionNetSnapshot {
                    sample_ms: now_ms,
                    rx_bytes: rx_bytes_total,
                    tx_bytes: tx_bytes_total,
                    if_name: current_if_name,
                    smoothed_rx_bps: 0.0,
                    smoothed_tx_bps: 0.0,
                });
            }
        }

        let net_up_str = format_speed(speed_tx);
        let net_down_str = format_speed(speed_rx);

        Ok(RemoteSystemMetrics {
            cpu,
            mem,
            disk,
            net_up: net_up_str,
            net_down: net_down_str,
            net_if: detected_if_name,
            net_if_options,
            cpu_model,
            cpu_cores,
            mem_total_bytes,
            mem_used_bytes,
            disk_total_bytes,
            disk_used_bytes,
        })
    })
    .await
    .map_err(|e| format!("执行错误: {}", e))?
}

fn format_speed(bps: f64) -> String {
    if bps < 1024.0 * 1024.0 {
        format!("{:.1} KB/s", bps / 1024.0)
    } else {
        format!("{:.2} MB/s", bps / (1024.0 * 1024.0))
    }
}

fn max_bounded(v: u32) -> u32 {
    if v > 100 { 100 } else { v }
}

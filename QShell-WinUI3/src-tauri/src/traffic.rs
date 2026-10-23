use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct IfaceCounters {
    rx: u64,
    tx: u64,
}

type IfaceBaseline = BTreeMap<String, IfaceCounters>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TrafficAuditParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    pub source: String,
    pub custom_command: String,
    pub reset_day: i32,
    pub reset_hour: i32,
    pub reset_minute: i32,
    pub reset_second: i32,
    pub direction: String,
    pub target_interface: String,
    pub traffic_used: String,
    pub traffic_used_unit: String,
    pub traffic_cycle_used_bytes: Option<u64>,
    pub traffic_delta_bytes: Option<u64>,
    pub last_raw_bytes: Option<u64>,
    pub traffic_counter_sig: String,
    pub traffic_boot_id: String,
    pub last_reset_timestamp: Option<f64>,
    pub traffic_iface_baseline: String,
    pub force_nic: bool,
}

#[derive(Debug, Serialize)]
pub struct TrafficAuditResult {
    pub traffic_cycle_used_bytes: u64,
    pub traffic_delta_bytes: u64,
    pub traffic_used: String,
    pub traffic_used_unit: String,
    pub last_raw_bytes: u64,
    pub traffic_counter_sig: String,
    pub traffic_boot_id: String,
    pub last_reset_timestamp: f64,
    pub traffic_source_effective: String,
    pub traffic_source_error: String,
    pub traffic_last_sync_ts: f64,
    pub traffic_iface_baseline: String,
}

#[derive(Debug, Default)]
struct Snapshot {
    boot_id: String,
    arch: String,
    proc_text: String,
    route_text: String,
    vnstat_text: String,
}

struct VnstatUsage {
    bytes: u64,
    coverage_complete: bool,
    stale: bool,
}

struct AuditState {
    cycle_used: u64,
    delta_bytes: u64,
    traffic_used: String,
    traffic_used_unit: String,
    last_raw: u64,
    sig: String,
    boot_id: String,
    iface_baseline: String,
    last_reset: f64,
    effective: String,
    error: String,
    last_sync: f64,
}

impl AuditState {
    fn from_params(p: &TrafficAuditParams) -> Self {
        Self {
            cycle_used: ensure_cycle_used(p),
            delta_bytes: p.traffic_delta_bytes.unwrap_or(0),
            traffic_used: p.traffic_used.clone(),
            traffic_used_unit: p.traffic_used_unit.clone(),
            last_raw: p.last_raw_bytes.unwrap_or(0),
            sig: p.traffic_counter_sig.clone(),
            boot_id: p.traffic_boot_id.clone(),
            iface_baseline: p.traffic_iface_baseline.clone(),
            last_reset: p.last_reset_timestamp.unwrap_or(0.0),
            effective: String::new(),
            error: String::new(),
            last_sync: 0.0,
        }
    }

    fn into_result(self) -> TrafficAuditResult {
        TrafficAuditResult {
            traffic_cycle_used_bytes: self.cycle_used,
            traffic_delta_bytes: self.delta_bytes,
            traffic_used: self.traffic_used,
            traffic_used_unit: self.traffic_used_unit,
            last_raw_bytes: self.last_raw,
            traffic_counter_sig: self.sig,
            traffic_boot_id: self.boot_id,
            last_reset_timestamp: self.last_reset,
            traffic_source_effective: self.effective,
            traffic_source_error: self.error,
            traffic_last_sync_ts: self.last_sync,
            traffic_iface_baseline: self.iface_baseline,
        }
    }

    fn set_used_from_cycle(&mut self) {
        let mult = traffic_mult(&self.traffic_used_unit);
        self.traffic_used = format!("{:.2}", self.cycle_used as f64 / mult);
    }

    fn finish_reset(&mut self, should_reset: bool, reset_ts: f64, audit_ts: f64) {
        if should_reset {
            self.last_reset = reset_ts;
        } else if self.last_reset <= 0.0 {
            self.last_reset = audit_ts;
        }
    }
}

fn traffic_mult(unit: &str) -> f64 {
    match unit.trim().to_uppercase().as_str() {
        "B" | "BYTES" => 1.0,
        "KB" => 1024.0,
        "MB" => 1024.0 * 1024.0,
        "GB" => 1024.0 * 1024.0 * 1024.0,
        "TB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => 1024.0 * 1024.0 * 1024.0,
    }
}

fn ensure_cycle_used(p: &TrafficAuditParams) -> u64 {
    if let Some(value) = p.traffic_cycle_used_bytes {
        return value;
    }
    let base_used = p.traffic_used.trim().parse::<f64>().unwrap_or(0.0).max(0.0);
    let delta = p.traffic_delta_bytes.unwrap_or(0) as f64;
    (base_used * traffic_mult(&p.traffic_used_unit) + delta).max(0.0) as u64
}

fn parse_iface_baseline(json: &str) -> IfaceBaseline {
    if json.trim().is_empty() {
        return IfaceBaseline::new();
    }
    serde_json::from_str(json).unwrap_or_default()
}

fn serialize_iface_baseline(map: &IfaceBaseline) -> String {
    serde_json::to_string(map).unwrap_or_default()
}

fn arch_is_64_bit(arch: &str) -> bool {
    let arch = arch.trim().to_lowercase();
    arch.is_empty()
        || arch.contains("64")
        || arch == "x86_64"
        || arch == "aarch64"
        || arch == "s390x"
        || arch == "ppc64le"
        || arch == "riscv64"
}

fn counter_delta(previous: u64, current: u64, _is_64_bit: bool) -> u64 {
    if current >= previous {
        return current - previous;
    }
    // 计数器回退意味着网卡被重建或计数被清零：从零开始算新流量。
    // 注意 /proc/net/dev 在所有 Linux（含 32 位内核）上都是 u64，
    // 不要按 u32 回绕补算，否则接口归零会被算成约 4GB 的虚增。
    current
}

fn counters_to_baseline(counters: &[(String, u64, u64)]) -> IfaceBaseline {
    counters
        .iter()
        .map(|(name, rx, tx)| {
            (
                name.clone(),
                IfaceCounters {
                    rx: *rx,
                    tx: *tx,
                },
            )
        })
        .collect()
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|d| d.pred_opt())
        .map(|d| d.day())
        .unwrap_or(28)
}

fn local_timestamp(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> f64 {
    let naive = NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|d| d.and_hms_opt(hour, minute, second));
    match naive {
        Some(naive) => match Local.from_local_datetime(&naive) {
            chrono::LocalResult::Single(dt) => dt.timestamp() as f64,
            chrono::LocalResult::Ambiguous(a, _) => a.timestamp() as f64,
            chrono::LocalResult::None => naive.and_utc().timestamp() as f64,
        },
        None => 0.0,
    }
}

fn reset_info(p: &TrafficAuditParams, now: &DateTime<Local>, now_ts: f64) -> (f64, bool) {
    let hour = p.reset_hour.clamp(0, 23) as u32;
    let minute = p.reset_minute.clamp(0, 59) as u32;
    let second = p.reset_second.clamp(0, 59) as u32;
    let day = p
        .reset_day
        .clamp(1, days_in_month(now.year(), now.month()) as i32) as u32;
    let mut reset_ts = local_timestamp(now.year(), now.month(), day, hour, minute, second);
    if now_ts < reset_ts {
        // 本月结算点还没到，说明最近一个结算点在上个月（短月按 clamp 处理）。
        // 这样即使结算当天没开应用，下个月首次审计时也能补结算，不会把两个周期合并。
        let (py, pm) = if now.month() == 1 {
            (now.year() - 1, 12)
        } else {
            (now.year(), now.month() - 1)
        };
        let pd = p.reset_day.clamp(1, days_in_month(py, pm) as i32) as u32;
        reset_ts = local_timestamp(py, pm, pd, hour, minute, second);
    }
    let last_reset = p.last_reset_timestamp.unwrap_or(0.0);
    let should_reset = last_reset > 0.0 && now_ts >= reset_ts && last_reset < reset_ts;
    (reset_ts, should_reset)
}

fn vnstat_cycle_start(p: &TrafficAuditParams, now: &DateTime<Local>) -> NaiveDate {
    let day = p
        .reset_day
        .clamp(1, days_in_month(now.year(), now.month()) as i32) as u32;
    let current = NaiveDate::from_ymd_opt(now.year(), now.month(), day)
        .unwrap_or_else(|| now.date_naive());
    if now.date_naive() >= current {
        current
    } else {
        let (py, pm) = if now.month() == 1 {
            (now.year() - 1, 12)
        } else {
            (now.year(), now.month() - 1)
        };
        let pd = p
            .reset_day
            .clamp(1, days_in_month(py, pm) as i32) as u32;
        NaiveDate::from_ymd_opt(py, pm, pd).unwrap_or(current)
    }
}

fn parse_snapshot(text: &str) -> Snapshot {
    let mut snapshot = Snapshot::default();

    let (boot_id, rest) = if let Some((boot_part, after_arch)) =
        text.split_once("__QSHELL_ARCH__")
    {
        snapshot.boot_id = boot_part.trim().to_string();
        if let Some((arch, rest)) = after_arch.split_once("__QSHELL_PROC__") {
            snapshot.arch = arch.trim().to_string();
            (snapshot.boot_id.clone(), rest.to_string())
        } else {
            return snapshot;
        }
    } else if let Some((boot_part, rest)) = text.split_once("__QSHELL_PROC__") {
        snapshot.boot_id = boot_part.trim().to_string();
        (snapshot.boot_id.clone(), rest.to_string())
    } else {
        snapshot.proc_text = text.to_string();
        return snapshot;
    };

    snapshot.boot_id = boot_id;
    let Some((proc_text, rest)) = rest.split_once("__QSHELL_ROUTE__") else {
        snapshot.proc_text = rest.to_string();
        return snapshot;
    };
    snapshot.proc_text = proc_text.to_string();
    let Some((route_text, vnstat_text)) = rest.split_once("__QSHELL_VNSTAT__") else {
        snapshot.route_text = rest.to_string();
        return snapshot;
    };
    snapshot.route_text = route_text.to_string();
    snapshot.vnstat_text = vnstat_text.to_string();
    snapshot
}

fn route_default_interface(route_text: &str) -> String {
    for line in route_text.lines() {
        if let Some(pos) = line.find("dev ") {
            let rest = &line[pos + 4..];
            if let Some(token) = rest.split_whitespace().next() {
                if !token.is_empty() {
                    return token.to_string();
                }
            }
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.first() == Some(&"0.0.0.0") && parts.len() >= 8 {
            return parts.last().unwrap().to_string();
        }
    }
    String::new()
}

fn parse_proc_counters(text: &str) -> Vec<(String, u64, u64)> {
    let mut counters = Vec::new();
    for line in text.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() < 9 {
            continue;
        }
        if let (Ok(rx), Ok(tx)) = (parts[0].parse::<u64>(), parts[8].parse::<u64>()) {
            counters.push((name.to_string(), rx, tx));
        }
    }
    counters
}

fn is_virtual_interface(name: &str) -> bool {
    let n = name.to_lowercase();
    if n == "lo" {
        return true;
    }
    const PREFIXES: [&str; 12] = [
        "docker", "veth", "br-", "virbr", "vmnet", "tun", "tap", "wg", "tailscale", "cni",
        "flannel", "lxc",
    ];
    PREFIXES.iter().any(|prefix| n.starts_with(prefix))
        || n.starts_with("ifb")
        || n.starts_with("dummy")
}

fn target_tokens(target: &str) -> Vec<String> {
    target
        .split(|c: char| c == ',' || c == ';' || c == '|' || c.is_whitespace())
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect()
}

fn is_all_token(token: &str) -> bool {
    matches!(token, "*" | "all" | "any")
}

fn interface_matches(name: &str, tokens: &[String]) -> bool {
    let name_lower = name.to_lowercase();
    for token in tokens {
        if is_all_token(token) {
            if !is_virtual_interface(name) {
                return true;
            }
            continue;
        }
        if let Some(prefix) = token.strip_suffix('*') {
            if !prefix.is_empty() && name_lower.starts_with(prefix) {
                return true;
            }
        } else if name_lower == *token {
            return true;
        }
    }
    false
}

fn select_proc_interfaces(
    counters: &[(String, u64, u64)],
    p: &TrafficAuditParams,
    route_text: &str,
) -> Vec<(String, u64, u64)> {
    let tokens = target_tokens(&p.target_interface);
    if !tokens.is_empty() {
        return counters
            .iter()
            .filter(|(name, _, _)| interface_matches(name, &tokens))
            .cloned()
            .collect();
    }

    let default_iface = route_default_interface(route_text);
    if !default_iface.is_empty() {
        let matched: Vec<_> = counters
            .iter()
            .filter(|(name, _, _)| name == &default_iface)
            .cloned()
            .collect();
        if !matched.is_empty() {
            return matched;
        }
    }

    // 自动模式保底：优先 eth0，再退到所有物理网卡
    if let Some(eth0) = counters
        .iter()
        .find(|(name, _, _)| name.eq_ignore_ascii_case("eth0"))
        .cloned()
    {
        return vec![eth0];
    }

    counters
        .iter()
        .filter(|(name, _, _)| !is_virtual_interface(name))
        .cloned()
        .collect()
}

fn sum_proc_bytes(
    selected: &[(String, u64, u64)],
    direction: &str,
) -> u64 {
    let direction = direction.to_lowercase();
    selected
        .iter()
        .map(|(_, rx, tx)| match direction.as_str() {
            "tx" => *tx,
            "rx" => *rx,
            _ => rx.saturating_add(*tx),
        })
        .fold(0u64, |acc, value| acc.saturating_add(value))
}

fn proc_signature(selected: &[(String, u64, u64)], direction: &str) -> String {
    let mut names: Vec<String> = selected.iter().map(|(name, _, _)| name.clone()).collect();
    names.sort();
    format!("{}|{}", names.join(","), direction.to_lowercase())
}

fn value_bytes(node: Option<&serde_json::Value>) -> u64 {
    let Some(node) = node else {
        return 0;
    };
    if let Some(bytes) = node.get("bytes") {
        return bytes
            .as_u64()
            .or_else(|| bytes.as_f64().map(|f| f.max(0.0) as u64))
            .unwrap_or(0);
    }
    node.as_u64()
        .or_else(|| node.as_f64().map(|f| f.max(0.0) as u64))
        .unwrap_or(0)
}

fn vnstat_period_bytes(traffic: Option<&serde_json::Value>, direction: &str) -> u64 {
    let rx = value_bytes(traffic.and_then(|t| t.get("rx")));
    let tx = value_bytes(traffic.and_then(|t| t.get("tx")));
    let total = value_bytes(traffic.and_then(|t| t.get("total")));
    match direction.to_lowercase().as_str() {
        "tx" => tx,
        "rx" => rx,
        _ => {
            if total > 0 {
                total
            } else {
                rx.saturating_add(tx)
            }
        }
    }
}

fn parse_vnstat_usage(
    text: &str,
    p: &TrafficAuditParams,
    now: &DateTime<Local>,
    route_text: &str,
) -> Option<VnstatUsage> {
    if text.trim().is_empty() || text.contains("__QSHELL_NO_VNSTAT__") {
        return None;
    }
    let start = text.find('{')?;
    let data: serde_json::Value = serde_json::from_str(&text[start..]).ok()?;
    let interfaces = data.get("interfaces")?.as_array()?;
    if interfaces.is_empty() {
        return None;
    }

    let tokens = target_tokens(&p.target_interface);
    let default_iface = route_default_interface(route_text);
    let mut selected: Vec<&serde_json::Value> = Vec::new();
    if !tokens.is_empty() {
        selected.extend(interfaces.iter().filter(|item| {
            item.get("name")
                .and_then(|n| n.as_str())
                .map(|name| interface_matches(name, &tokens))
                .unwrap_or(false)
        }));
    } else if !default_iface.is_empty() {
        selected.extend(
            interfaces
                .iter()
                .filter(|item| item.get("name").and_then(|n| n.as_str()) == Some(default_iface.as_str())),
        );
        if selected.is_empty() {
            selected.extend(
                interfaces
                    .iter()
                    .filter(|item| {
                        item.get("name")
                            .and_then(|n| n.as_str())
                            .map(|name| !is_virtual_interface(name))
                            .unwrap_or(false)
                    }),
            );
        }
    } else {
        selected.extend(
            interfaces
                .iter()
                .filter(|item| {
                    item.get("name")
                        .and_then(|n| n.as_str())
                        .map(|name| !is_virtual_interface(name))
                        .unwrap_or(false)
                }),
        );
    }
    if selected.is_empty() {
        return None;
    }

    let cycle_start = vnstat_cycle_start(p, now);
    let mut total: u64 = 0;
    let mut found_period = false;

    for interface in &selected {
        let Some(traffic) = interface.get("traffic") else {
            continue;
        };
        let days = traffic.get("day").and_then(|v| v.as_array());
        if let Some(days) = days {
            if !days.is_empty() {
                for entry in days {
                    let Some(date) = entry.get("date") else {
                        continue;
                    };
                    let (Some(y), Some(m), Some(d)) = (
                        date.get("year").and_then(|v| v.as_i64()),
                        date.get("month").and_then(|v| v.as_i64()),
                        date.get("day").and_then(|v| v.as_i64()),
                    ) else {
                        continue;
                    };
                    let Some(entry_date) =
                        NaiveDate::from_ymd_opt(y as i32, m as u32, d as u32)
                    else {
                        continue;
                    };
                    if entry_date >= cycle_start {
                        let traffic_node = entry.get("traffic").unwrap_or(entry);
                        total = total.saturating_add(vnstat_period_bytes(
                            Some(traffic_node),
                            &p.direction,
                        ));
                        found_period = true;
                    }
                }
                continue;
            }
        }

        if let Some(months) = traffic.get("month").and_then(|v| v.as_array()) {
            for entry in months {
                let Some(date) = entry.get("date") else {
                    continue;
                };
                let (Some(y), Some(m)) = (
                    date.get("year").and_then(|v| v.as_i64()),
                    date.get("month").and_then(|v| v.as_i64()),
                ) else {
                    continue;
                };
                if y as i32 == now.year() && m as u32 == now.month() {
                    let traffic_node = entry.get("traffic").unwrap_or(entry);
                    total = total.saturating_add(vnstat_period_bytes(
                        Some(traffic_node),
                        &p.direction,
                    ));
                    found_period = true;
                    break;
                }
            }
        }
    }

    if !found_period {
        return None;
    }

    let cycle_start_ts = local_timestamp(
        cycle_start.year(),
        cycle_start.month(),
        cycle_start.day(),
        // 结算时刻不一定是 0 点；用配置的时分秒，否则每周期会多算"0 点到结算点"的流量。
        p.reset_hour.clamp(0, 23) as u32,
        p.reset_minute.clamp(0, 59) as u32,
        p.reset_second.clamp(0, 59) as u32,
    );
    let now_ts = now.timestamp() as f64;
    let mut coverage_complete = true;
    let mut latest_updated: f64 = 0.0;
    for interface in &selected {
        if let Some(created) = interface
            .get("created")
            .and_then(|value| value.get("timestamp"))
            .and_then(|value| value.as_f64())
        {
            if created > 0.0 && created > cycle_start_ts {
                coverage_complete = false;
            }
        }
        if let Some(updated) = interface
            .get("updated")
            .and_then(|value| value.get("timestamp"))
            .and_then(|value| value.as_f64())
        {
            if updated > latest_updated {
                latest_updated = updated;
            }
        }
    }

    let stale = latest_updated > 0.0 && now_ts - latest_updated > 15.0 * 60.0;
    Some(VnstatUsage {
        bytes: total,
        coverage_complete,
        stale,
    })
}

fn parse_custom_usage(text: &str) -> Option<u64> {
    let re = regex::Regex::new(
        r"(?i)^\s*([0-9]+(?:\.[0-9]+)?)\s*(B|BYTES|KB|MB|GB|TB)?\s*$",
    )
    .ok()?;
    for line in text.lines() {
        let Some(caps) = re.captures(line) else {
            continue;
        };
        let value = caps.get(1)?.as_str().parse::<f64>().ok()?;
        let unit = caps
            .get(2)
            .map(|m| m.as_str())
            .unwrap_or("B");
        return Some((value.max(0.0) * traffic_mult(unit)) as u64);
    }
    None
}

fn open_session(p: &TrafficAuditParams) -> Result<ssh2::Session, String> {
    let addrs: Vec<_> = (p.host.as_str(), p.port)
        .to_socket_addrs()
        .map_err(|e| format!("解析主机地址失败: {}", e))?
        .collect();
    let mut tcp = None;
    let mut last_error = String::new();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            Ok(stream) => {
                tcp = Some(stream);
                break;
            }
            Err(e) => last_error = e.to_string(),
        }
    }
    let tcp = tcp.ok_or_else(|| {
        if last_error.is_empty() {
            "无法连接到目标主机".to_string()
        } else {
            format!("连接目标主机失败: {}", last_error)
        }
    })?;
    let _ = tcp.set_nodelay(true);

    let mut sess = ssh2::Session::new().map_err(|e| format!("初始化 SSH 会话失败: {}", e))?;
    sess.set_tcp_stream(tcp);
    sess.set_timeout(10_000);
    let _ = sess.method_pref(
        ssh2::MethodType::Kex,
        "ecdh-sha2-nistp256,ecdh-sha2-nistp384,ecdh-sha2-nistp521,diffie-hellman-group14-sha256,diffie-hellman-group16-sha512,diffie-hellman-group-exchange-sha256,diffie-hellman-group14-sha1",
    );
    let _ = sess.method_pref(
        ssh2::MethodType::HostKey,
        "rsa-sha2-512,rsa-sha2-256,ecdsa-sha2-nistp256,ecdsa-sha2-nistp384,ecdsa-sha2-nistp521,ssh-rsa",
    );
    let _ = sess.method_pref(
        ssh2::MethodType::CryptCs,
        "aes128-ctr,aes192-ctr,aes256-ctr,aes128-cbc,aes256-cbc,3des-cbc",
    );
    let _ = sess.method_pref(
        ssh2::MethodType::CryptSc,
        "aes128-ctr,aes192-ctr,aes256-ctr,aes128-cbc,aes256-cbc,3des-cbc",
    );
    sess.handshake().map_err(|e| format!("SSH 握手失败: {}", e))?;

    if let Some(key_path) = p.private_key.as_deref() {
        if !key_path.trim().is_empty() {
            let path = std::path::Path::new(key_path);
            if path.is_file() {
                let _ = sess.userauth_pubkey_file(
                    &p.username,
                    None,
                    path,
                    p.passphrase.as_deref(),
                );
            }
        }
    }
    if !sess.authenticated() {
        if let Some(password) = p.password.as_deref() {
            sess.userauth_password(&p.username, password)
                .map_err(|e| format!("密码认证失败: {}", e))?;
        }
    }
    if !sess.authenticated() {
        return Err("身份认证未通过，请检查用户名、密码或私钥".to_string());
    }
    Ok(sess)
}

fn run_remote_script(p: &TrafficAuditParams, script: &str, timeout_secs: u64) -> Result<String, String> {
    let sess = open_session(p)?;
    sess.set_timeout(timeout_secs.saturating_mul(1000) as u32);
    let mut channel = sess
        .channel_session()
        .map_err(|e| format!("创建会话通道失败: {}", e))?;
    channel
        .exec(script)
        .map_err(|e| format!("执行远程命令失败: {}", e))?;

    let mut output = String::new();
    let mut buf = [0u8; 8192];
    loop {
        match channel.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => output.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(_) => break,
        }
    }
    let _ = channel.wait_close();
    Ok(output)
}

fn probe_snapshot(p: &TrafficAuditParams) -> Result<Snapshot, String> {
    let script = concat!(
        "cat /proc/sys/kernel/random/boot_id 2>/dev/null; ",
        "echo __QSHELL_ARCH__; uname -m 2>/dev/null; ",
        "echo __QSHELL_PROC__; cat /proc/net/dev 2>/dev/null; ",
        "echo __QSHELL_ROUTE__; ",
        "ip route show default 2>/dev/null | head -n 5; ",
        "route -n 2>/dev/null | grep '^0.0.0.0' | head -n 5; ",
        "echo __QSHELL_VNSTAT__; ",
        "if command -v vnstat >/dev/null 2>&1; then vnstat --json 2>/dev/null; ",
        "else echo __QSHELL_NO_VNSTAT__; fi; ",
        // 末尾哨兵：只有读到它才说明整段快照完整；否则中途超时/截断会被误当完整数据。
        "echo __QSHELL_DONE__"
    );
    let text = run_remote_script(p, script, 10)?;
    let Some((payload, _)) = text.split_once("__QSHELL_DONE__") else {
        // 截断快照绝不能用来覆盖网卡基线：网卡从基线里消失后再出现时增量会被记 0，
        // 造成永久漏计且不报错，这里直接如实失败。
        return Err("远端流量快照不完整（输出被截断）".to_string());
    };
    Ok(parse_snapshot(payload))
}

fn apply_proc_baseline(
    state: &mut AuditState,
    snapshot: &Snapshot,
    p: &TrafficAuditParams,
) -> bool {
    let counters = parse_proc_counters(&snapshot.proc_text);
    let selected = select_proc_interfaces(&counters, p, &snapshot.route_text);
    if selected.is_empty() {
        return false;
    }
    state.last_raw = sum_proc_bytes(&selected, &p.direction);
    state.sig = proc_signature(&selected, &p.direction);
    state.boot_id = snapshot.boot_id.clone();
    state.iface_baseline = serialize_iface_baseline(&counters_to_baseline(&counters));
    state.delta_bytes = 0;
    true
}

fn apply_external_measurement(
    state: &mut AuditState,
    p: &TrafficAuditParams,
    snapshot: Option<&Snapshot>,
    measured: u64,
    reset_ts: f64,
    should_reset: bool,
    audit_ts: f64,
    authoritative: bool,
) {
    if let Some(snapshot) = snapshot {
        apply_proc_baseline(state, snapshot, p);
    } else {
        state.last_raw = 0;
        state.sig.clear();
        state.boot_id.clear();
        state.iface_baseline.clear();
        state.delta_bytes = 0;
    }

    // vnstat 的周期绝对值可信度高于本地记录：即使低于本地值也直接采用。
    // 自定义命令仍保留高水位逻辑，避免偶发的小值把已用流量改小。
    state.error.clear();
    if authoritative || should_reset {
        state.cycle_used = measured;
    } else if measured < state.cycle_used {
        // keep the previous higher value
        state.error = "远程统计值低于本地记录，已保留较高值".to_string();
    } else {
        state.cycle_used = measured;
    }
    state.delta_bytes = 0;
    state.set_used_from_cycle();
    state.finish_reset(should_reset, reset_ts, audit_ts);
    state.last_sync = audit_ts;
}

fn direction_delta(rx_delta: u64, tx_delta: u64, direction: &str) -> u64 {
    match direction.to_lowercase().as_str() {
        "rx" => rx_delta,
        "tx" => tx_delta,
        _ => rx_delta.saturating_add(tx_delta),
    }
}

fn apply_proc_measurement(
    state: &mut AuditState,
    p: &TrafficAuditParams,
    snapshot: &Snapshot,
    reset_ts: f64,
    should_reset: bool,
    audit_ts: f64,
    fallback_error: &str,
) -> Result<(), String> {
    let counters = parse_proc_counters(&snapshot.proc_text);
    let selected = select_proc_interfaces(&counters, p, &snapshot.route_text);
    if selected.is_empty() {
        return Err("未找到可统计的网卡".to_string());
    }

    let current_raw = sum_proc_bytes(&selected, &p.direction);
    let signature = proc_signature(&selected, &p.direction);
    let current_baseline = counters_to_baseline(&counters);
    let previous_baseline = parse_iface_baseline(&state.iface_baseline);
    let has_previous_baseline = !previous_baseline.is_empty();
    let is_64_bit = arch_is_64_bit(&snapshot.arch);
    let boot_changed = !state.boot_id.is_empty()
        && !snapshot.boot_id.is_empty()
        && state.boot_id != snapshot.boot_id;

    let delta_bytes = if should_reset {
        0
    } else if !has_previous_baseline {
        // 旧配置迁移：还没有逐网卡基线时，沿用原来的单总量差值逻辑
        if !state.sig.is_empty() && state.sig != signature {
            0
        } else if boot_changed {
            current_raw
        } else if state.last_raw > 0 {
            counter_delta(state.last_raw, current_raw, is_64_bit)
        } else {
            0
        }
    } else if boot_changed {
        // 主机重启后计数器归零：当前计数就是重启以来的流量
        current_raw
    } else {
        // 逐网卡差值：某块网卡新增/删除不再让整台主机丢一个采样周期
        let mut delta = 0u64;
        for (name, rx, tx) in &selected {
            if let Some(previous) = previous_baseline.get(name) {
                let rx_delta = counter_delta(previous.rx, *rx, is_64_bit);
                let tx_delta = counter_delta(previous.tx, *tx, is_64_bit);
                delta = delta.saturating_add(direction_delta(
                    rx_delta,
                    tx_delta,
                    &p.direction,
                ));
            }
        }
        delta
    };

    state.cycle_used = if should_reset {
        0
    } else {
        state.cycle_used.saturating_add(delta_bytes)
    };
    state.delta_bytes = if should_reset {
        0
    } else {
        state.delta_bytes.saturating_add(delta_bytes)
    };
    state.last_raw = current_raw;
    state.sig = signature;
    state.boot_id = snapshot.boot_id.clone();
    state.iface_baseline = serialize_iface_baseline(&current_baseline);
    state.set_used_from_cycle();
    state.finish_reset(should_reset, reset_ts, audit_ts);

    if should_reset {
        state.traffic_used = "0".to_string();
    }
    state.error = fallback_error.to_string();
    state.last_sync = audit_ts;
    Ok(())
}

pub fn audit_host_traffic_sync(p: TrafficAuditParams) -> Result<TrafficAuditResult, String> {
    let now = Local::now();
    let audit_ts = now.timestamp() as f64;
    let (reset_ts, should_reset) = reset_info(&p, &now, audit_ts);
    let source = p.source.to_lowercase();
    let mut state = AuditState::from_params(&p);

    let snapshot_result = probe_snapshot(&p);
    let mut snapshot_error = String::new();
    let snapshot = match snapshot_result {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            snapshot_error = error;
            None
        }
    };

    let mut error = String::new();

    if source == "custom" {
        let command = p.custom_command.trim();
        if command.is_empty() {
            error = "自定义命令为空".to_string();
        } else {
            match run_remote_script(&p, command, 8) {
                Ok(output) => {
                    if let Some(measured) = parse_custom_usage(&output) {
                        apply_external_measurement(
                            &mut state,
                            &p,
                            snapshot.as_ref(),
                            measured,
                            reset_ts,
                            should_reset,
                            audit_ts,
                            false,
                        );
                        state.effective = "custom".to_string();
                        return Ok(state.into_result());
                    }
                    error = "自定义命令未输出有效流量数字".to_string();
                }
                Err(e) => error = format!("自定义命令失败：{}", e),
            }
        }
    }

    if !p.force_nic && (source == "auto" || source == "vnstat") {
        if let Some(snapshot) = snapshot.as_ref() {
            if let Some(usage) =
                parse_vnstat_usage(&snapshot.vnstat_text, &p, &now, &snapshot.route_text)
            {
                if usage.stale {
                    error = "vnstat 数据已过期，已回退本地计数器".to_string();
                } else if !usage.coverage_complete {
                    // vnstat 是中途安装的，当前周期绝对值不完整，不能作为锚点前的总量
                    error = "vnstat 历史不完整，已回退本地计数器".to_string();
                } else {
                    apply_external_measurement(
                        &mut state,
                        &p,
                        Some(snapshot),
                        usage.bytes,
                        reset_ts,
                        should_reset,
                        audit_ts,
                        true,
                    );
                    state.effective = "vnstat".to_string();
                    return Ok(state.into_result());
                }
            }
        }
        if error.is_empty() {
            error = "vnstat 未安装或无可用数据，已回退本地计数器".to_string();
        }
    }

    if let Some(snapshot) = snapshot.as_ref() {
        match apply_proc_measurement(
            &mut state,
            &p,
            snapshot,
            reset_ts,
            should_reset,
            audit_ts,
            &error,
        ) {
            Ok(()) => {
                state.effective = "proc".to_string();
                return Ok(state.into_result());
            }
            Err(proc_error) => {
                if !proc_error.is_empty() {
                    error = proc_error;
                }
            }
        }
    } else if error.is_empty() {
        error = if snapshot_error.is_empty() {
            "无法读取远程流量数据".to_string()
        } else {
            snapshot_error
        };
    }

    state.effective = "none".to_string();
    state.error = error;
    state.last_sync = audit_ts;
    Ok(state.into_result())
}

#[tauri::command]
pub async fn audit_host_traffic(params: TrafficAuditParams) -> Result<TrafficAuditResult, String> {
    tokio::task::spawn_blocking(move || audit_host_traffic_sync(params))
        .await
        .map_err(|e| format!("流量审计任务异常: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_params() -> TrafficAuditParams {
        TrafficAuditParams {
            host: "127.0.0.1".to_string(),
            port: 22,
            username: "root".to_string(),
            password: None,
            private_key: None,
            passphrase: None,
            source: "auto".to_string(),
            custom_command: String::new(),
            reset_day: 1,
            reset_hour: 0,
            reset_minute: 0,
            reset_second: 0,
            direction: "both".to_string(),
            target_interface: String::new(),
            traffic_used: "0".to_string(),
            traffic_used_unit: "GB".to_string(),
            traffic_cycle_used_bytes: None,
            traffic_delta_bytes: None,
            last_raw_bytes: None,
            traffic_counter_sig: String::new(),
            traffic_boot_id: String::new(),
            last_reset_timestamp: None,
            traffic_iface_baseline: String::new(),
            force_nic: false,
        }
    }

    #[test]
    fn parses_custom_usage_with_units() {
        let gb = parse_custom_usage("123.45 GB\n").unwrap();
        assert_eq!(gb, (123.45_f64 * 1024.0 * 1024.0 * 1024.0) as u64);
        assert_eq!(parse_custom_usage("1048576").unwrap(), 1_048_576);
        assert_eq!(parse_custom_usage("512 MB").unwrap(), 512 * 1024 * 1024);
        assert!(parse_custom_usage("not a number").is_none());
    }

    #[test]
    fn parses_proc_counters_and_selects_interface() {
        let text = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n  eth0: 1000 10 20 30 40 0 0 0 2000 5 6 7 8 0 0 0\n";
        let counters = parse_proc_counters(text);
        assert_eq!(counters.len(), 1);
        assert_eq!(counters[0].0, "eth0");
        assert_eq!(counters[0].1, 1000);
        assert_eq!(counters[0].2, 2000);

        let p = test_params();
        let selected = select_proc_interfaces(
            &counters,
            &p,
            "default via 192.168.1.1 dev eth0 proto static",
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(sum_proc_bytes(&selected, "both"), 3000);
        assert_eq!(sum_proc_bytes(&selected, "tx"), 2000);
        assert_eq!(sum_proc_bytes(&selected, "rx"), 1000);
    }

    #[test]
    fn parses_vnstat_days_since_cycle_start() {
        let json = r#"{
          "interfaces": [{
            "name": "eth0",
            "traffic": {
              "day": [
                {"date": {"year": 2026, "month": 9, "day": 30}, "traffic": {"rx": 111, "tx": 222, "total": 333}},
                {"date": {"year": 2026, "month": 10, "day": 2}, "traffic": {"rx": 600, "tx": 400, "total": 1000}},
                {"date": {"year": 2026, "month": 10, "day": 3}, "traffic": {"rx": 200, "tx": 300, "total": 500}}
              ]
            }
          }]
        }"#;
        let now = Local
            .with_ymd_and_hms(2026, 10, 19, 12, 0, 0)
            .single()
            .unwrap();
        let p = test_params();
        let measured =
            parse_vnstat_usage(json, &p, &now, "default via 192.168.1.1 dev eth0").unwrap();
        assert_eq!(measured.bytes, 1500);
        assert!(measured.coverage_complete);
        assert!(!measured.stale);

        // vnstat 2.x 的日/月条目有时把 rx/tx 直接放在条目顶层
        let flat_json = r#"{
          "interfaces": [{
            "name": "eth0",
            "traffic": {
              "day": [
                {"date": {"year": 2026, "month": 10, "day": 2}, "rx": 600, "tx": 400},
                {"date": {"year": 2026, "month": 10, "day": 3}, "rx": 200, "tx": 300}
              ]
            }
          }]
        }"#;
        let measured_flat =
            parse_vnstat_usage(flat_json, &p, &now, "default via 192.168.1.1 dev eth0").unwrap();
        assert_eq!(measured_flat.bytes, 1500);
    }

    #[test]
    fn selects_multiple_interfaces() {
        let counters = vec![
            ("eth0".to_string(), 100, 200),
            ("eth1".to_string(), 300, 400),
            ("lo".to_string(), 1, 1),
            ("docker0".to_string(), 1, 1),
        ];
        let mut p = test_params();

        p.target_interface = "eth0, eth1".to_string();
        let names: Vec<String> = select_proc_interfaces(&counters, &p, "")
            .into_iter()
            .map(|(name, _, _)| name)
            .collect();
        assert_eq!(names, vec!["eth0", "eth1"]);

        p.target_interface = "eth*".to_string();
        assert_eq!(
            select_proc_interfaces(&counters, &p, "").len(),
            2
        );

        p.target_interface = "*".to_string();
        assert_eq!(
            select_proc_interfaces(&counters, &p, "").len(),
            2
        );

        p.target_interface = String::new();
        let selected =
            select_proc_interfaces(&counters, &p, "default via 1.1.1.1 dev eth1");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].0, "eth1");

        // 没有默认路由时回退 eth0
        let fallback = select_proc_interfaces(&counters, &p, "");
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].0, "eth0");
    }

    #[test]
    fn external_measurement_keeps_higher_value() {
        let p = test_params();
        let mut state = AuditState::from_params(&p);
        state.cycle_used = 1000;
        state.traffic_used = "1000".to_string();
        state.traffic_used_unit = "B".to_string();

        apply_external_measurement(&mut state, &p, None, 500, 0.0, false, 100.0, false);
        assert_eq!(state.cycle_used, 1000);
        assert!(state.error.contains("保留较高值"));

        apply_external_measurement(&mut state, &p, None, 2000, 0.0, false, 200.0, false);
        assert_eq!(state.cycle_used, 2000);
        assert!(state.error.is_empty());

        apply_external_measurement(&mut state, &p, None, 100, 300.0, true, 300.0, false);
        assert_eq!(state.cycle_used, 100);
        assert!(state.error.is_empty());
    }

    #[test]
    fn vnstat_absolute_value_overrides_local_higher_value() {
        let p = test_params();
        let mut state = AuditState::from_params(&p);
        state.cycle_used = 1000;
        state.traffic_used = "1000".to_string();
        state.traffic_used_unit = "B".to_string();

        // vnstat 是绝对口径，即使低于本地记录也必须采用 vnstat 的值
        apply_external_measurement(&mut state, &p, None, 500, 0.0, false, 100.0, true);
        assert_eq!(state.cycle_used, 500);
        assert_eq!(state.traffic_used, "500.00");
        assert!(state.error.is_empty());
    }

    #[test]
    fn parses_snapshot_markers() {
        let text = "boot-123\n__QSHELL_ARCH__\nx86_64\n__QSHELL_PROC__\n  eth0: 1 2 3 4 5 6 7 8 9\n__QSHELL_ROUTE__\ndefault via 1.1.1.1 dev eth0\n__QSHELL_VNSTAT__\n{\"interfaces\":[]}";
        let snapshot = parse_snapshot(text);
        assert_eq!(snapshot.boot_id, "boot-123");
        assert_eq!(snapshot.arch, "x86_64");
        assert!(snapshot.proc_text.contains("eth0"));
        assert!(snapshot.route_text.contains("dev eth0"));
        assert!(snapshot.vnstat_text.contains("interfaces"));
        assert_eq!(
            route_default_interface(&snapshot.route_text),
            "eth0"
        );
    }

    #[test]
    fn counter_delta_handles_wrap_and_reset() {
        assert_eq!(counter_delta(100, 150, false), 50);
        // /proc/net/dev 是 u64，接口归零后按"从零开始"处理，不再做 u32 回绕补算。
        assert_eq!(counter_delta(u32::MAX as u64 - 10, 100, false), 100);
        assert_eq!(counter_delta(1000, 100, true), 100);
        assert_eq!(counter_delta(1000, 1000, true), 0);
    }

    #[test]
    fn per_interface_baseline_survives_interface_removal() {
        let p = test_params();
        let mut state = AuditState::from_params(&p);
        state.cycle_used = 0;
        state.traffic_used_unit = "B".to_string();

        let first = Snapshot {
            boot_id: "boot-a".to_string(),
            arch: "x86_64".to_string(),
            proc_text: "  eth0: 100 0 0 0 0 0 0 0 200 0 0 0 0 0 0 0\n  eth1: 500 0 0 0 0 0 0 0 600 0 0 0 0 0 0 0\n".to_string(),
            route_text: "default via 1.1.1.1 dev eth0".to_string(),
            vnstat_text: String::new(),
        };
        apply_proc_measurement(&mut state, &p, &first, 0.0, false, 100.0, "").unwrap();
        assert_eq!(state.cycle_used, 0);
        assert!(!state.iface_baseline.is_empty());

        let second = Snapshot {
            boot_id: "boot-a".to_string(),
            arch: "x86_64".to_string(),
            proc_text: "  eth0: 150 0 0 0 0 0 0 0 280 0 0 0 0 0 0 0\n".to_string(),
            route_text: "default via 1.1.1.1 dev eth0".to_string(),
            vnstat_text: String::new(),
        };
        apply_proc_measurement(&mut state, &p, &second, 0.0, false, 200.0, "").unwrap();
        // eth0: +50 rx +80 tx = 130；eth1 被移除不影响本轮
        assert_eq!(state.cycle_used, 130);
    }

    #[test]
    fn detects_incomplete_and_stale_vnstat() {
        let now = Local
            .with_ymd_and_hms(2026, 10, 19, 12, 0, 0)
            .single()
            .unwrap();
        let cycle_start_ts = local_timestamp(2026, 10, 1, 0, 0, 0) as i64;
        let p = test_params();
        let incomplete = format!(
            r#"{{
              "interfaces": [{{
                "name": "eth0",
                "created": {{"timestamp": {}}},
                "updated": {{"timestamp": {}}},
                "traffic": {{"day": [{{"date": {{"year": 2026, "month": 10, "day": 2}}, "rx": 100, "tx": 200}}]}}
              }}]
            }}"#,
            cycle_start_ts + 86400,
            now.timestamp()
        );
        let usage =
            parse_vnstat_usage(&incomplete, &p, &now, "default via 1.1.1.1 dev eth0").unwrap();
        assert!(!usage.coverage_complete);
        assert!(!usage.stale);

        let stale = format!(
            r#"{{
              "interfaces": [{{
                "name": "eth0",
                "created": {{"timestamp": {}}},
                "updated": {{"timestamp": {}}},
                "traffic": {{"day": [{{"date": {{"year": 2026, "month": 10, "day": 2}}, "rx": 100, "tx": 200}}]}}
              }}]
            }}"#,
            cycle_start_ts - 86400,
            now.timestamp() - 3600
        );
        let usage = parse_vnstat_usage(&stale, &p, &now, "default via 1.1.1.1 dev eth0").unwrap();
        assert!(usage.coverage_complete);
        assert!(usage.stale);
    }
}

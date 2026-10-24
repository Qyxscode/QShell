use serde::Serialize;
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::net::IpAddr;
use std::process::Stdio;
use std::time::Duration;
use tauri::Emitter;

#[derive(Serialize, Clone)]
pub struct TraceHop {
    pub hop: u32,
    pub ip: String,
    pub rtt_ms: Vec<f64>,
    pub timeout: bool,
    pub location: Option<String>,
    pub isp: Option<String>,
}

/// 解析 tracert 的单行输出：
///   1    <1 ms    <1 ms    <1 ms  192.168.1.1
///   2     *        *        *     请求超时。
fn parse_line(line: &str) -> Option<TraceHop> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let hop_no: u32 = tokens[0].trim_end_matches('.').parse().ok()?;
    tokens.remove(0);

    let mut rtts: Vec<f64> = Vec::new();
    let mut ip = String::new();
    let mut timeout = false;
    let mut i = 0usize;
    while i < tokens.len() {
        let token = tokens[i];
        if token == "*" {
            timeout = true;
            i += 1;
            continue;
        }
        if token.eq_ignore_ascii_case("ms") {
            if i > 0 {
                let num = tokens[i - 1];
                let is_less = num.starts_with('<');
                let cleaned = num.trim_start_matches('<').trim_end_matches("ms");
                if let Ok(v) = cleaned.parse::<f64>() {
                    rtts.push(if is_less { 0.5 } else { v });
                }
            }
            i += 1;
            continue;
        }
        if token.contains('.') || token.contains(':') {
            let candidate = token.trim_matches(|c| c == '[' || c == ']');
            if !candidate.is_empty()
                && candidate
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() || c == '.' || c == ':')
                && candidate.parse::<IpAddr>().is_ok()
            {
                ip = candidate.to_string();
            }
        }
        i += 1;
    }

    Some(TraceHop {
        hop: hop_no,
        ip,
        rtt_ms: rtts,
        timeout,
        location: None,
        isp: None,
    })
}

/// 通过 ip-api.com 查询单个 IP 的归属地与运营商（失败返回 None）。
fn lookup_location(ip: &str) -> Option<(String, String)> {
    if ip.parse::<IpAddr>().is_err() {
        return None;
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .ok()?;
    let url = format!(
        "http://ip-api.com/json/{}?fields=status,country,regionName,city,isp&lang=zh-CN",
        ip
    );
    let resp: serde_json::Value = client.get(&url).send().ok()?.json().ok()?;
    if resp.get("status").and_then(|v| v.as_str()) != Some("success") {
        return None;
    }
    let country = resp.get("country").and_then(|v| v.as_str()).unwrap_or("");
    let region = resp.get("regionName").and_then(|v| v.as_str()).unwrap_or("");
    let city = resp.get("city").and_then(|v| v.as_str()).unwrap_or("");
    let isp = resp.get("isp").and_then(|v| v.as_str()).unwrap_or("");
    let location = if !city.is_empty() {
        format!("{} · {}", country, city)
    } else if !region.is_empty() {
        format!("{} · {}", country, region)
    } else {
        country.to_string()
    };
    if location.trim().is_empty() {
        None
    } else {
        Some((location, isp.to_string()))
    }
}

/// 流式启动路由追踪：每解析到一跳立即 emit `trace-hop`，
/// 位置查询完成后 emit `trace-hop-location`，全部结束 emit `trace-done`。
#[tauri::command]
pub async fn start_trace_route(
    app: tauri::AppHandle,
    request_id: String,
    host: String,
) -> Result<(), String> {
    let target = host.trim().to_string();
    if target.is_empty() {
        return Err("缺少目标地址".to_string());
    }
    if !target
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':')
    {
        return Err("目标地址包含非法字符".to_string());
    }

    tokio::task::spawn_blocking(move || {
        let command_line = format!("chcp 65001 >nul && tracert -d -h 30 -w 1500 {}", target);
        let mut child = {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x08000000;
                std::process::Command::new("cmd")
                    .args(["/c", &command_line])
                    .stdout(Stdio::piped())
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn()
                    .map_err(|e| format!("执行 tracert 失败: {}", e))?
            }
            #[cfg(not(windows))]
            {
                std::process::Command::new("sh")
                    .args(["-c", &command_line])
                    .stdout(Stdio::piped())
                    .spawn()
                    .map_err(|e| format!("执行 tracert 失败: {}", e))?
            }
        };

        let stdout = match child.stdout.take() {
            Some(v) => v,
            None => return Err("无法读取 tracert 输出".to_string()),
        };
        let reader = BufReader::new(stdout);
        let mut hop_count = 0usize;

        for line in reader.lines() {
            let line = match line {
                Ok(v) => v,
                Err(_) => break,
            };
            if let Some(hop) = parse_line(&line) {
                hop_count += 1;
                let _ = app.emit(
                    "trace-hop",
                    json!({ "requestId": &request_id, "hop": hop }),
                );

                // 位置查询放到独立线程，查到后再推送一次更新
                let ip = hop.ip.clone();
                if !ip.is_empty() {
                    let app_clone = app.clone();
                    let request_clone = request_id.clone();
                    std::thread::spawn(move || {
                        if let Some((location, isp)) = lookup_location(&ip) {
                            let _ = app_clone.emit(
                                "trace-hop-location",
                                json!({
                                    "requestId": request_clone,
                                    "ip": ip,
                                    "location": location,
                                    "isp": isp,
                                }),
                            );
                        }
                    });
                }
            }
        }

        let _ = child.wait();
        let _ = app.emit(
            "trace-done",
            json!({ "requestId": request_id, "hopCount": hop_count }),
        );
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("路由追踪任务失败: {}", e))?
}

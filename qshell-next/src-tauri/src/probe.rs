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

//! Whois 与 DNS 查询。
//!
//! 两者都直接说协议，不依赖系统命令：
//! - Whois = TCP 43 端口，发 `域名\r\n`，读回纯文本
//! - DNS   = 自建 UDP 53 查询报文，解析响应里的资源记录
//!
//! 之所以不用 `nslookup` / `whois` 子进程：nslookup 的输出格式随系统语言和
//! 版本变（中文 Windows 全是本地化标签），解析文本比解析二进制报文脆弱得多。

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

// ===================== Whois =====================

/// 常见 TLD → whois 服务器。没收录的按 `whois.nic.<tld>` 拼。
fn whois_server_for(query: &str) -> String {
    let q = query.trim().trim_end_matches('.').to_lowercase();
    // IP 反查：走 ARIN，它会用 ReferralServer 把我们指到对应的 RIR
    // （APNIC / RIPE / LACNIC / AFRINIC）。不用 iana：实测国内网络连不上。
    if q.parse::<IpAddr>().is_ok() {
        return "whois.arin.net".to_string();
    }
    let tld = q.rsplit('.').next().unwrap_or("");
    let server = match tld {
        "com" | "net" => "whois.verisign-grs.com",
        "org" => "whois.pir.org",
        "cn" => "whois.cnnic.cn",
        "io" => "whois.nic.io",
        "co" => "whois.nic.co",
        "me" => "whois.nic.me",
        "tv" => "tvwhois.verisign-grs.com",
        "cc" => "ccwhois.verisign-grs.com",
        "info" => "whois.nic.info",
        "xyz" => "whois.nic.xyz",
        "top" => "whois.nic.top",
        "vip" => "whois.nic.vip",
        "club" => "whois.nic.club",
        "site" => "whois.nic.site",
        "online" => "whois.nic.online",
        "tech" => "whois.nic.tech",
        "store" => "whois.nic.store",
        "app" | "dev" => "whois.nic.google",
        "jp" => "whois.jprs.jp",
        "kr" => "whois.kr",
        "hk" => "whois.hkirc.hk",
        "tw" => "whois.twnic.net.tw",
        "sg" => "whois.sgnic.sg",
        "uk" => "whois.nic.uk",
        "de" => "whois.denic.de",
        "fr" => "whois.nic.fr",
        "ru" => "whois.tcinet.ru",
        "au" => "whois.auda.org.au",
        "ca" => "whois.cira.ca",
        "us" => "whois.nic.us",
        "eu" => "whois.eu",
        other => return format!("whois.nic.{}", other),
    };
    server.to_string()
}

/// 向指定 whois 服务器查询一次，返回原始文本。
fn whois_fetch(server: &str, query: &str, timeout: Duration) -> Result<String, String> {
    let addr = (server, 43u16)
        .to_socket_addrs_first()
        .ok_or_else(|| format!("无法解析 whois 服务器 {}", server))?;
    let mut stream =
        TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("连接 {} 失败: {}", server, e))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| format!("设置超时失败: {}", e))?;
    stream
        .set_write_timeout(Some(timeout))
        .ok();

    // WHOIS 协议要求 CRLF 结尾
    let payload = format!("{}\r\n", query.trim());
    stream
        .write_all(payload.as_bytes())
        .map_err(|e| format!("发送查询失败: {}", e))?;
    stream.flush().ok();

    // 服务器不会关连接前给长度，只能读到 EOF；限制上限防止无限读取
    const MAX_BYTES: usize = 512 * 1024;
    let mut out: Vec<u8> = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                out.extend_from_slice(&chunk[..n]);
                if out.len() >= MAX_BYTES {
                    out.truncate(MAX_BYTES);
                    break;
                }
            }
            Err(e) => {
                // 读超时：把已经拿到的内容返回，比直接报错有用
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut
                {
                    break;
                }
                if out.is_empty() {
                    return Err(format!("读取响应失败: {}", e));
                }
                break;
            }
        }
    }

    if out.is_empty() {
        return Err("whois 服务器没有返回内容".to_string());
    }
    // 绝大多数 whois 服务器返回 UTF-8；cnnic 也是。个别老的用 GBK，
    // 这里只做有损解码，不引入编码库（数字和英文部分永远正确）
    Ok(String::from_utf8_lossy(&out).to_string())
}

/// 从 whois 文本里找出推荐的下一跳服务器（refer / ReferralServer / Registrar WHOIS Server）。
fn whois_referral(text: &str) -> Option<String> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        if let Some(idx) = lower.find("referralserver:") {
            let mut v = line[idx + "referralserver:".len()..].trim().to_string();
            // 形如 whois://whois.arin.net
            if let Some(rest) = v.strip_prefix("whois://") {
                v = rest.to_string();
            }
            v = v.trim_end_matches('/').to_string();
            if !v.is_empty() && v.contains('.') {
                return Some(v);
            }
        }
        if let Some(idx) = lower.find("refer:") {
            let v = line[idx + "refer:".len()..].trim().to_string();
            if !v.is_empty() && v.contains('.') && !v.contains(' ') {
                return Some(v);
            }
        }
        if let Some(idx) = lower.find("registrar whois server:") {
            let v = line[idx + "registrar whois server:".len()..].trim().to_string();
            if !v.is_empty() && v.contains('.') && !v.contains(' ') {
                return Some(v);
            }
        }
    }
    None
}

/// 查询域名或 IP 的 whois 信息。
///
/// 会跟随一次 refer：注册局 / ARIN 通常会指向真正持有数据的服务器。
#[tauri::command]
pub async fn whois_lookup(query: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let q = query.trim().to_string();
        if q.is_empty() {
            return Err("请输入域名或 IP".to_string());
        }

        let timeout = Duration::from_millis(8000);
        let first_server = whois_server_for(&q);
        let first = match whois_fetch(&first_server, &q, timeout) {
            Ok(t) => t,
            Err(e) => {
                // 没收录的 TLD（whois.nic.<tld> 不存在）再试一次 IANA 兜底。
                // 注意 IANA 在国内网络经常连不上，所以它只当最后手段。
                if first_server != "whois.iana.org" {
                    match whois_fetch("whois.iana.org", &q, Duration::from_millis(4000)) {
                        Ok(t) => t,
                        Err(_) => return Err(e),
                    }
                } else {
                    return Err(e);
                }
            }
        };

        // 响应里往往只有一行 refer，跟一次能拿到真正的注册信息。
        // 跳过 whois.iana.org：它是「这是 IANA 自己的域名」的回指，跟过去没有新信息。
        if let Some(next) = whois_referral(&first) {
            if next != first_server && next != "whois.iana.org" {
                if let Ok(second) = whois_fetch(&next, &q, timeout) {
                    if !second.trim().is_empty() {
                        return Ok(second);
                    }
                }
            }
        }

        Ok(first)
    })
    .await
    .map_err(|e| format!("whois 任务异常: {}", e))?
}

/// 把 "host:port" 解析成第一个地址的小工具（避免手写 ToSocketAddrs 样板）。
trait FirstSocketAddr {
    fn to_socket_addrs_first(&self) -> Option<SocketAddr>;
}

impl FirstSocketAddr for (&str, u16) {
    fn to_socket_addrs_first(&self) -> Option<SocketAddr> {
        use std::net::ToSocketAddrs;
        self.to_socket_addrs().ok()?.next()
    }
}

// ===================== DNS =====================

/// 结果行，前端直接渲染成表格。
#[derive(serde::Serialize)]
pub struct DnsAnswer {
    pub name: String,
    pub rtype: String,
    pub ttl: u32,
    pub value: String,
}

fn qtype_code(kind: &str) -> Option<u16> {
    let k = kind.trim().to_uppercase();
    Some(match k.as_str() {
        "A" => 1,
        "NS" => 2,
        "CNAME" => 5,
        "SOA" => 6,
        "PTR" => 12,
        "MX" => 15,
        "TXT" => 16,
        "AAAA" => 28,
        "SRV" => 33,
        "CAA" => 257,
        "ANY" => 255,
        _ => return None,
    })
}

fn qtype_name(code: u16) -> String {
    match code {
        1 => "A".to_string(),
        2 => "NS".to_string(),
        5 => "CNAME".to_string(),
        6 => "SOA".to_string(),
        12 => "PTR".to_string(),
        15 => "MX".to_string(),
        16 => "TXT".to_string(),
        28 => "AAAA".to_string(),
        33 => "SRV".to_string(),
        257 => "CAA".to_string(),
        other => format!("#{}", other),
    }
}

/// 构造一个标准 DNS 查询报文（递归查询 RD=1）。
fn build_dns_query(domain: &str, qtype: u16) -> Result<Vec<u8>, String> {
    let name = domain.trim().trim_end_matches('.');
    if name.is_empty() {
        return Err("域名为空".to_string());
    }
    let id: u16 = rand::random();
    let mut buf: Vec<u8> = Vec::with_capacity(64);
    buf.extend_from_slice(&id.to_be_bytes());
    buf.extend_from_slice(&0x0100u16.to_be_bytes()); // flags: RD=1
    buf.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    buf.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    buf.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    buf.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT

    for label in name.split('.') {
        if label.is_empty() {
            return Err("域名里有空的标签，检查是否多写了点".to_string());
        }
        if label.len() > 63 {
            return Err(format!("域名标签过长: {}", label));
        }
        buf.push(label.len() as u8);
        buf.extend_from_slice(label.as_bytes());
    }
    buf.push(0); // 根
    buf.extend_from_slice(&qtype.to_be_bytes());
    buf.extend_from_slice(&1u16.to_be_bytes()); // IN
    Ok(buf)
}

/// 读一个 DNS 域名，支持 0xC0 压缩指针。
fn read_dns_name(data: &[u8], pos: &mut usize) -> Result<String, String> {
    let mut labels: Vec<String> = Vec::new();
    let mut p = *pos;
    let mut end: Option<usize> = None;
    let mut jumps = 0usize;

    loop {
        if p >= data.len() {
            return Err("DNS 报文越界（域名）".to_string());
        }
        let len = data[p];
        if len == 0 {
            p += 1;
            if end.is_none() {
                end = Some(p);
            }
            break;
        }
        if len & 0xC0 == 0xC0 {
            if p + 1 >= data.len() {
                return Err("DNS 报文越界（指针）".to_string());
            }
            let ptr = (((len & 0x3F) as usize) << 8) | data[p + 1] as usize;
            if end.is_none() {
                end = Some(p + 2);
            }
            jumps += 1;
            if jumps > 64 {
                return Err("DNS 名称压缩指针成环".to_string());
            }
            p = ptr;
            continue;
        }
        let l = len as usize;
        p += 1;
        if p + l > data.len() {
            return Err("DNS 报文越界（标签）".to_string());
        }
        labels.push(String::from_utf8_lossy(&data[p..p + l]).to_string());
        p += l;
    }

    *pos = end.unwrap_or(p);
    if labels.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(labels.join("."))
    }
}

struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn u16(&mut self) -> Result<u16, String> {
        if self.pos + 2 > self.data.len() {
            return Err("DNS 报文越界".to_string());
        }
        let v = u16::from_be_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32, String> {
        if self.pos + 4 > self.data.len() {
            return Err("DNS 报文越界".to_string());
        }
        let v = u32::from_be_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }
}
/// 解析一条资源记录的 rdata。
fn parse_rdata(data: &[u8], rtype: u16, start: usize, rdlen: usize) -> String {
    let end = start + rdlen;
    if end > data.len() {
        return "<rdata 越界>".to_string();
    }
    let mut p = start;
    match rtype {
        1 => {
            // A
            if rdlen == 4 {
                Ipv4Addr::new(data[start], data[start + 1], data[start + 2], data[start + 3]).to_string()
            } else {
                "<A 记录长度异常>".to_string()
            }
        }
        28 => {
            // AAAA
            if rdlen == 16 {
                let mut oct = [0u8; 16];
                oct.copy_from_slice(&data[start..start + 16]);
                Ipv6Addr::from(oct).to_string()
            } else {
                "<AAAA 记录长度异常>".to_string()
            }
        }
        2 | 5 | 12 => {
            // NS / CNAME / PTR
            match read_dns_name(data, &mut p) {
                Ok(n) => n,
                Err(_) => "<名称解析失败>".to_string(),
            }
        }
        15 => {
            // MX: preference(2) + exchange
            if rdlen < 3 {
                return "<MX 记录长度异常>".to_string();
            }
            let pref = u16::from_be_bytes([data[start], data[start + 1]]);
            p = start + 2;
            match read_dns_name(data, &mut p) {
                Ok(n) => format!("{} {}", pref, n),
                Err(_) => "<MX 名称解析失败>".to_string(),
            }
        }
        16 => {
            // TXT: 一串 length-prefixed 字符串
            let mut parts: Vec<String> = Vec::new();
            while p < end {
                let l = data[p] as usize;
                p += 1;
                if p + l > end {
                    break;
                }
                parts.push(String::from_utf8_lossy(&data[p..p + l]).to_string());
                p += l;
            }
            parts.join(" ")
        }
        6 => {
            // SOA: mname rname serial refresh retry expire minimum
            match read_dns_name(data, &mut p) {
                Ok(mname) => {
                    if let Ok(rname) = read_dns_name(data, &mut p) {
                        let mut nums: Vec<u32> = Vec::new();
                        for _ in 0..5 {
                            if p + 4 > end {
                                break;
                            }
                            let v = u32::from_be_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
                            nums.push(v);
                            p += 4;
                        }
                        format!("{} {} {}", mname, rname, nums.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" "))
                    } else {
                        mname
                    }
                }
                Err(_) => "<SOA 解析失败>".to_string(),
            }
        }
        33 => {
            // SRV: priority(2) weight(2) port(2) target
            if rdlen < 7 {
                return "<SRV 记录长度异常>".to_string();
            }
            let prio = u16::from_be_bytes([data[start], data[start + 1]]);
            let weight = u16::from_be_bytes([data[start + 2], data[start + 3]]);
            let port = u16::from_be_bytes([data[start + 4], data[start + 5]]);
            p = start + 6;
            match read_dns_name(data, &mut p) {
                Ok(target) => format!("{} {} {} {}", prio, weight, port, target),
                Err(_) => "<SRV 目标解析失败>".to_string(),
            }
        }
        257 => {
            // CAA: flags(1) tag_len(1) tag value
            if rdlen < 2 {
                return "<CAA 记录长度异常>".to_string();
            }
            let flags = data[start];
            let tag_len = data[start + 1] as usize;
            if start + 2 + tag_len > end {
                return "<CAA 记录长度异常>".to_string();
            }
            let tag = String::from_utf8_lossy(&data[start + 2..start + 2 + tag_len]).to_string();
            let value = String::from_utf8_lossy(&data[start + 2 + tag_len..end]).to_string();
            format!("{} {} {}", flags, tag, value)
        }
        _ => {
            // 未知类型：给十六进制预览，方便用户自己判断
            let raw = &data[start..end];
            let preview: Vec<String> = raw.iter().take(24).map(|b| format!("{:02x}", b)).collect();
            format!("0x{}", preview.join(""))
        }
    }
}

/// 解析 DNS 响应报文，返回答案记录列表。
fn parse_dns_response(data: &[u8]) -> Result<Vec<DnsAnswer>, String> {
    if data.len() < 12 {
        return Err("DNS 响应过短".to_string());
    }
    let mut r = ByteReader::new(data);
    let _id = r.u16()?;
    let flags = r.u16()?;
    let rcode = (flags & 0x000F) as u8;
    let qdcount = r.u16()?;
    let ancount = r.u16()?;
    let nscount = r.u16()?;
    let arcount = r.u16()?;

    if flags & 0x0200 != 0 {
        return Err("DNS 服务器返回了截断报文".to_string());
    }

    // 跳过 QUESTION 区
    for _ in 0..qdcount {
        let _name = read_dns_name(data, &mut r.pos)?;
        let _qtype = r.u16()?;
        let _qclass = r.u16()?;
    }

    if rcode != 0 {
        let msg = match rcode {
            1 => "查询格式错误",
            2 => "DNS 服务器内部错误",
            3 => "域名不存在（NXDOMAIN）",
            4 => "DNS 服务器不支持该查询类型",
            5 => "DNS 服务器拒绝查询（REFUSED）",
            _ => "未知错误",
        };
        return Err(format!("DNS 查询失败: {} (rcode={})", msg, rcode));
    }

    let total = ancount as usize + nscount as usize + arcount as usize;
    let mut answers: Vec<DnsAnswer> = Vec::with_capacity(total);
    for _ in 0..total {
        let name = read_dns_name(data, &mut r.pos)?;
        let rtype = r.u16()?;
        let _rclass = r.u16()?;
        let ttl = r.u32()?;
        let rdlen = r.u16()? as usize;
        let rdata_start = r.pos;
        let value = parse_rdata(data, rtype, rdata_start, rdlen);
        // 无论解析是否成功，都按 rdlen 前进，保证后续记录能读对
        r.pos = rdata_start + rdlen;
        if r.pos > data.len() {
            return Err("DNS 报文越界（rdata）".to_string());
        }
        answers.push(DnsAnswer {
            name,
            rtype: qtype_name(rtype),
            ttl,
            value,
        });
    }
    Ok(answers)
}

/// 读系统配置的 DNS 服务器（Windows 从注册表读，其他平台返回空）。
#[cfg(target_os = "windows")]
fn system_dns_servers() -> Vec<IpAddr> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let mut out: Vec<IpAddr> = Vec::new();
    let base = "SYSTEM\\CurrentControlSet\\Services\\Tcpip\\Parameters\\Interfaces";
    let Ok(hklm) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(base) else {
        return out;
    };
    for key_name in hklm.enum_keys().flatten() {
        let Ok(sub) = hklm.open_subkey(&key_name) else {
            continue;
        };
        // DhcpNameServer 是 DHCP 下发的，NameServer 是静态配置的，两个都要看
        for value_name in ["DhcpNameServer", "NameServer"] {
            if let Ok(raw) = sub.get_value::<String, _>(value_name) {
                for tok in raw.split(|c| c == ' ' || c == ',' || c == ';') {
                    let t = tok.trim();
                    if let Ok(ip) = t.parse::<IpAddr>() {
                        if !ip.is_unspecified() && !out.contains(&ip) {
                            out.push(ip);
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(not(target_os = "windows"))]
fn system_dns_servers() -> Vec<IpAddr> {
    Vec::new()
}

fn udp_dns_query(server: IpAddr, packet: &[u8], timeout: Duration) -> Result<Vec<u8>, String> {
    let bind_addr = if server.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
    let socket = UdpSocket::bind(bind_addr).map_err(|e| format!("创建 DNS socket 失败: {}", e))?;
    socket
        .set_read_timeout(Some(timeout))
        .map_err(|e| format!("设置超时失败: {}", e))?;
    socket
        .send_to(packet, SocketAddr::new(server, 53))
        .map_err(|e| format!("发送 DNS 查询失败: {}", e))?;
    let mut buf = vec![0u8; 4096];
    let (n, _) = socket
        .recv_from(&mut buf)
        .map_err(|e| format!("接收 DNS 响应失败: {}", e))?;
    buf.truncate(n);
    Ok(buf)
}

/// 查询域名（或 PTR 反查 IP）的 DNS 记录。
#[tauri::command]
pub async fn dns_lookup(domain: String, record_type: String) -> Result<Vec<DnsAnswer>, String> {
    tokio::task::spawn_blocking(move || {
        let raw_domain = domain.trim().to_string();
        if raw_domain.is_empty() {
            return Err("请输入域名".to_string());
        }
        let qtype = qtype_code(&record_type).ok_or_else(|| format!("不支持的记录类型: {}", record_type))?;

        // PTR 查询允许直接填 IP，自动转成 in-addr.arpa / ip6.arpa
        let query_name = if qtype == 12 {
            match raw_domain.parse::<IpAddr>() {
                Ok(IpAddr::V4(v4)) => {
                    let o = v4.octets();
                    format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0])
                }
                Ok(IpAddr::V6(v6)) => {
                    let mut s = String::new();
                    for b in v6.octets().iter().rev() {
                        s.push_str(&format!("{:x}.{:x}.", b & 0x0F, b >> 4));
                    }
                    s.push_str("ip6.arpa");
                    s
                }
                Err(_) => raw_domain.clone(),
            }
        } else {
            raw_domain.clone()
        };

        let packet = build_dns_query(&query_name, qtype)?;

        // 先问系统 DNS，再逐个试公共 DNS
        let mut servers = system_dns_servers();
        for fallback in ["223.5.5.5", "119.29.29.29", "1.1.1.1", "8.8.8.8"] {
            if let Ok(ip) = fallback.parse::<IpAddr>() {
                if !servers.contains(&ip) {
                    servers.push(ip);
                }
            }
        }

        let mut last_err = String::from("没有可用的 DNS 服务器");
        // 有的服务器「响应成功但记录为空」——国内 DNS 对 github.com 的 AAAA
        // 就会这样。这种情况要换下一个服务器问，不能当成最终答案。
        let mut saw_empty_success = false;
        for server in servers {
            match udp_dns_query(server, &packet, Duration::from_millis(2000)) {
                Ok(resp) => match parse_dns_response(&resp) {
                    Ok(mut answers) => {
                        // 只保留用户要的类型（ANY 除外），避免 CNAME 链把结果撑乱
                        if qtype != 255 {
                            answers.retain(|a| a.rtype == qtype_name(qtype));
                        }
                        if answers.is_empty() {
                            saw_empty_success = true;
                            last_err = format!("{} 没有返回 {} 记录", server, qtype_name(qtype));
                            continue;
                        }
                        return Ok(answers);
                    }
                    Err(e) => {
                        // NXDOMAIN / REFUSED 是确定性结论，换服务器问也是同样结果
                        if e.contains("NXDOMAIN") || e.contains("REFUSED") {
                            return Err(e);
                        }
                        last_err = format!("{} 响应解析失败: {}", server, e);
                        continue;
                    }
                },
                Err(e) => {
                    last_err = format!("{}: {}", server, e);
                    continue;
                }
            }
        }
        // 所有服务器都答"成功但没有该记录"，这是正常的空结果，不是错误
        if saw_empty_success {
            return Ok(Vec::new());
        }
        Err(last_err)
    })
    .await
    .map_err(|e| format!("DNS 任务异常: {}", e))?
}

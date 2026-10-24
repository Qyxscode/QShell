use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::io::Write;
use parking_lot::Mutex;
use crate::generic_terminal::telnet_codec::TelnetCodec;

pub enum ProtocolType {
    Telnet,
    Serial,
    Local,
    Raw,
    Rlogin,
}

pub struct GenericSession {
    pub instance_id: u64,
    pub protocol: ProtocolType,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub tcp_stream: Option<TcpStream>,
    pub stop_flag: Arc<AtomicBool>,
    pub telnet_codec: Option<Arc<Mutex<TelnetCodec>>>,
    pub newline_mode: String, // "crlf", "cr", "lf"
    pub local_echo: bool,
    /// 本地终端（ConPTY）的 master 与子进程，用于 resize / 结束进程
    pub pty_master: Option<Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>>,
    pub pty_child: Option<Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>>,
}

impl GenericSession {
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(stream) = &self.tcp_stream {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        if let Some(child) = &self.pty_child {
            let _ = child.lock().kill();
        }
    }

    pub fn resize_pty(&self, cols: u16, rows: u16) {
        if let Some(master) = &self.pty_master {
            let _ = master.lock().resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    pub fn resize_telnet(&self, cols: u16, rows: u16) -> Option<Vec<u8>> {
        if let Some(codec_lock) = &self.telnet_codec {
            let mut codec = codec_lock.lock();
            codec.cols = cols;
            codec.rows = rows;
            if codec.naws_enabled {
                return Some(TelnetCodec::build_naws_subneg(cols, rows));
            }
        }
        None
    }
}

pub struct Utf8ChunkDecoder {
    remainder: Vec<u8>,
}

impl Utf8ChunkDecoder {
    pub fn new() -> Self {
        Self { remainder: Vec::with_capacity(4) }
    }

    pub fn decode(&mut self, chunk: &[u8]) -> String {
        let mut data = std::mem::take(&mut self.remainder);
        data.extend_from_slice(chunk);
        let mut out = String::new();
        let mut slice = &data[..];

        loop {
            match std::str::from_utf8(slice) {
                Ok(valid) => {
                    out.push_str(valid);
                    break;
                }
                Err(e) => {
                    let valid_len = e.valid_up_to();
                    if valid_len > 0 {
                        let valid = unsafe { std::str::from_utf8_unchecked(&slice[..valid_len]) };
                        out.push_str(valid);
                    }
                    match e.error_len() {
                        Some(invalid_len) => {
                            out.push('\u{FFFD}');
                            slice = &slice[valid_len + invalid_len..];
                        }
                        None => {
                            self.remainder = slice[valid_len..].to_vec();
                            break;
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod utf8_tests {
    use super::Utf8ChunkDecoder;

    #[test]
    fn decodes_ascii() {
        let mut d = Utf8ChunkDecoder::new();
        assert_eq!(d.decode(b"hello"), "hello");
    }

    #[test]
    fn decodes_complete_multibyte() {
        let mut d = Utf8ChunkDecoder::new();
        assert_eq!(d.decode("你好".as_bytes()), "你好");
    }

    #[test]
    fn carries_incomplete_multibyte_tail() {
        let bytes = "你好".as_bytes();
        let mut d = Utf8ChunkDecoder::new();
        assert_eq!(d.decode(&bytes[..1]), "");
        assert_eq!(d.decode(&bytes[1..4]), "你");
        assert_eq!(d.decode(&bytes[4..]), "好");
    }

    #[test]
    fn decodes_ascii_before_incomplete_tail() {
        let bytes = "ok你好".as_bytes();
        let mut d = Utf8ChunkDecoder::new();
        assert_eq!(d.decode(&bytes[..3]), "ok");
        assert_eq!(d.decode(&bytes[3..]), "你好");
    }

    #[test]
    fn replaces_invalid_byte_and_keeps_following_text() {
        let mut d = Utf8ChunkDecoder::new();
        assert_eq!(d.decode(&[0xFF, b'A']), "\u{FFFD}A");
    }
}

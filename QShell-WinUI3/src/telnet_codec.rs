pub const IAC: u8 = 255;
pub const DONT: u8 = 254;
pub const DO: u8 = 253;
pub const WONT: u8 = 252;
pub const WILL: u8 = 251;
pub const SB: u8 = 250;
pub const SE: u8 = 240;

pub const OPT_ECHO: u8 = 1;
pub const OPT_SGA: u8 = 3;
pub const OPT_TTYPE: u8 = 24;
pub const OPT_NAWS: u8 = 31;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    Iac,
    Will,
    Wont,
    Do,
    Dont,
    SubReq,
    SubIac,
}

pub struct TelnetCodec {
    state: State,
    sub_buf: Vec<u8>,
    pub cols: u16,
    pub rows: u16,
    pub naws_enabled: bool,
    ttype_enabled: bool,
    sga_local_enabled: bool,
    sga_remote_enabled: bool,
    echo_enabled: bool,
    /// TTYPE 协商时回复给服务器的终端类型
    term_name: String,
}

impl TelnetCodec {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self::with_term(cols, rows, "xterm-256color")
    }

    pub fn with_term(cols: u16, rows: u16, term: &str) -> Self {
        Self {
            state: State::Normal,
            sub_buf: Vec::with_capacity(64),
            cols,
            rows,
            naws_enabled: false,
            ttype_enabled: false,
            sga_local_enabled: false,
            sga_remote_enabled: false,
            echo_enabled: false,
            term_name: if term.trim().is_empty() || term.trim() == "auto" {
                "xterm-256color".to_string()
            } else {
                term.trim().to_string()
            },
        }
    }

    pub fn build_naws_subneg(cols: u16, rows: u16) -> Vec<u8> {
        let mut msg = vec![IAC, SB, OPT_NAWS];
        let w_high = (cols >> 8) as u8;
        let w_low = (cols & 0xFF) as u8;
        let h_high = (rows >> 8) as u8;
        let h_low = (rows & 0xFF) as u8;
        for &b in &[w_high, w_low, h_high, h_low] {
            msg.push(b);
            if b == IAC {
                msg.push(IAC);
            }
        }
        msg.push(IAC);
        msg.push(SE);
        msg
    }
}

impl TelnetCodec {
    pub fn process_input(&mut self, input: &[u8], reply: &mut Vec<u8>) -> Vec<u8> {
        let mut clean_data = Vec::with_capacity(input.len());
        for &b in input {
            match self.state {
                State::Normal => {
                    if b == IAC {
                        self.state = State::Iac;
                    } else {
                        clean_data.push(b);
                    }
                }
                State::Iac => match b {
                    IAC => {
                        clean_data.push(IAC);
                        self.state = State::Normal;
                    }
                    WILL => self.state = State::Will,
                    WONT => self.state = State::Wont,
                    DO => self.state = State::Do,
                    DONT => self.state = State::Dont,
                    SB => {
                        self.sub_buf.clear();
                        self.state = State::SubReq;
                    }
                    _ => self.state = State::Normal,
                },
                State::Will => {
                    self.handle_will(b, reply);
                    self.state = State::Normal;
                }
                State::Wont => {
                    self.handle_wont(b, reply);
                    self.state = State::Normal;
                }
                State::Do => {
                    self.handle_do(b, reply);
                    self.state = State::Normal;
                }
                State::Dont => {
                    self.handle_dont(b, reply);
                    self.state = State::Normal;
                }
                State::SubReq => {
                    if b == IAC {
                        self.state = State::SubIac;
                    } else {
                        self.sub_buf.push(b);
                    }
                }
                State::SubIac => {
                    if b == SE {
                        self.handle_subneg(reply);
                        self.state = State::Normal;
                    } else if b == IAC {
                        self.sub_buf.push(IAC);
                        self.state = State::SubReq;
                    } else {
                        self.state = State::Normal;
                    }
                }
            }
        }
        clean_data
    }
}

impl TelnetCodec {
    fn handle_do(&mut self, opt: u8, reply: &mut Vec<u8>) {
        match opt {
            OPT_TTYPE => {
                if !self.ttype_enabled {
                    self.ttype_enabled = true;
                    reply.extend_from_slice(&[IAC, WILL, OPT_TTYPE]);
                }
            }
            OPT_NAWS => {
                if !self.naws_enabled {
                    self.naws_enabled = true;
                    reply.extend_from_slice(&[IAC, WILL, OPT_NAWS]);
                    reply.extend_from_slice(&Self::build_naws_subneg(self.cols, self.rows));
                }
            }
            OPT_SGA => {
                if !self.sga_local_enabled {
                    self.sga_local_enabled = true;
                    reply.extend_from_slice(&[IAC, WILL, OPT_SGA]);
                }
            }
            _ => reply.extend_from_slice(&[IAC, WONT, opt]),
        }
    }

    fn handle_dont(&mut self, opt: u8, reply: &mut Vec<u8>) {
        match opt {
            OPT_NAWS => self.naws_enabled = false,
            OPT_TTYPE => self.ttype_enabled = false,
            OPT_SGA => self.sga_local_enabled = false,
            _ => {}
        }
        reply.extend_from_slice(&[IAC, WONT, opt]);
    }

    fn handle_will(&mut self, opt: u8, reply: &mut Vec<u8>) {
        match opt {
            OPT_ECHO => {
                if !self.echo_enabled {
                    self.echo_enabled = true;
                    reply.extend_from_slice(&[IAC, DO, OPT_ECHO]);
                }
            }
            OPT_SGA => {
                if !self.sga_remote_enabled {
                    self.sga_remote_enabled = true;
                    reply.extend_from_slice(&[IAC, DO, OPT_SGA]);
                }
            }
            _ => reply.extend_from_slice(&[IAC, DONT, opt]),
        }
    }

    fn handle_wont(&mut self, opt: u8, reply: &mut Vec<u8>) {
        match opt {
            OPT_ECHO => self.echo_enabled = false,
            OPT_SGA => self.sga_remote_enabled = false,
            _ => {}
        }
        reply.extend_from_slice(&[IAC, DONT, opt]);
    }

    fn handle_subneg(&mut self, reply: &mut Vec<u8>) {
        if self.sub_buf.first() == Some(&OPT_TTYPE) && self.sub_buf.get(1) == Some(&1) {
            reply.extend_from_slice(&[IAC, SB, OPT_TTYPE, 0]);
            reply.extend_from_slice(self.term_name.as_bytes());
            reply.extend_from_slice(&[IAC, SE]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaped_iac_is_data() {
        let mut codec = TelnetCodec::new(80, 24);
        let mut reply = Vec::new();
        let data = codec.process_input(&[IAC, IAC, b'A'], &mut reply);
        assert_eq!(data, vec![IAC, b'A']);
        assert!(reply.is_empty());
    }

    #[test]
    fn handles_do_naws_and_sends_window_size() {
        let mut codec = TelnetCodec::new(120, 40);
        let mut reply = Vec::new();
        let data = codec.process_input(&[IAC, DO, OPT_NAWS], &mut reply);
        assert!(data.is_empty());
        assert!(codec.naws_enabled);
        assert_eq!(&reply[..3], &[IAC, WILL, OPT_NAWS]);
        assert!(reply.ends_with(&[IAC, SE]));
    }

    #[test]
    fn answers_ttype_request() {
        let mut codec = TelnetCodec::new(80, 24);
        let mut reply = Vec::new();
        codec.process_input(
            &[IAC, SB, OPT_TTYPE, 1, IAC, SE],
            &mut reply,
        );
        assert!(reply.windows(3).any(|w| w == [IAC, SB, OPT_TTYPE]));
        assert!(reply.windows(14).any(|w| w == b"xterm-256color"));
    }

    #[test]
    fn iac_state_survives_split_chunks() {
        let mut codec = TelnetCodec::new(80, 24);
        let mut reply = Vec::new();
        assert!(codec.process_input(&[IAC], &mut reply).is_empty());
        codec.process_input(&[DO, OPT_NAWS], &mut reply);
        assert!(reply.windows(3).any(|w| w == [IAC, WILL, OPT_NAWS]));
    }

    #[test]
    fn repeated_negotiation_is_idempotent() {
        let mut codec = TelnetCodec::new(80, 24);
        let mut reply = Vec::new();

        codec.process_input(&[IAC, DO, OPT_NAWS], &mut reply);
        assert!(!reply.is_empty());
        reply.clear();
        codec.process_input(&[IAC, DO, OPT_NAWS], &mut reply);
        assert!(reply.is_empty());

        codec.process_input(&[IAC, WILL, OPT_ECHO], &mut reply);
        assert_eq!(reply, vec![IAC, DO, OPT_ECHO]);
        reply.clear();
        codec.process_input(&[IAC, WILL, OPT_ECHO], &mut reply);
        assert!(reply.is_empty());
    }

    #[test]
    fn sga_local_and_remote_are_tracked_separately() {
        let mut codec = TelnetCodec::new(80, 24);
        let mut reply = Vec::new();
        codec.process_input(&[IAC, DO, OPT_SGA], &mut reply);
        assert_eq!(reply, vec![IAC, WILL, OPT_SGA]);
        reply.clear();
        codec.process_input(&[IAC, WILL, OPT_SGA], &mut reply);
        assert_eq!(reply, vec![IAC, DO, OPT_SGA]);
    }

    #[test]
    fn rejects_unknown_do_and_will() {
        let mut codec = TelnetCodec::new(80, 24);
        let mut reply = Vec::new();
        codec.process_input(&[IAC, DO, 99], &mut reply);
        assert_eq!(reply, vec![IAC, WONT, 99]);

        reply.clear();
        codec.process_input(&[IAC, WILL, 98], &mut reply);
        assert_eq!(reply, vec![IAC, DONT, 98]);
    }
}

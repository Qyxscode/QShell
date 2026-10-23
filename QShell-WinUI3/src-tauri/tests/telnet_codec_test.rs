#[path = "../src/telnet_codec.rs"]
mod telnet_codec;

use telnet_codec::{TelnetCodec, IAC, WILL, DO, OPT_TTYPE, OPT_NAWS, SB, SE};

#[test]
fn test_telnet_iac_filtering() {
    let mut codec = TelnetCodec::new(80, 24);
    let mut reply = Vec::new();
    let raw = [b'h', b'e', b'l', b'l', b'o', IAC, DO, OPT_TTYPE, b'!', b'\r', b'\n'];
    let clean = codec.process_input(&raw, &mut reply);

    assert_eq!(clean, b"hello!\r\n");
    assert_eq!(reply, vec![IAC, WILL, OPT_TTYPE]);
}

#[test]
fn test_telnet_naws_negotiation() {
    let mut codec = TelnetCodec::new(120, 40);
    let mut reply = Vec::new();
    let raw = [IAC, DO, OPT_NAWS];
    let clean = codec.process_input(&raw, &mut reply);

    assert!(clean.is_empty());
    assert!(codec.naws_enabled);
    assert_eq!(&reply[0..3], &[IAC, WILL, OPT_NAWS]);
    assert_eq!(&reply[3..6], &[IAC, SB, OPT_NAWS]);
}

#[test]
fn test_telnet_ttype_subnegotiation() {
    let mut codec = TelnetCodec::new(80, 24);
    let mut reply = Vec::new();
    let raw = [IAC, SB, OPT_TTYPE, 1, IAC, SE];
    codec.process_input(&raw, &mut reply);

    assert_eq!(&reply[0..4], &[IAC, SB, OPT_TTYPE, 0]);
    assert_eq!(&reply[4..18], b"xterm-256color");
    assert_eq!(&reply[18..20], &[IAC, SE]);
}

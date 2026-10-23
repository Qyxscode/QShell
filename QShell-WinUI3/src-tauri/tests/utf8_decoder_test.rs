use qshell_lib::generic_terminal::generic_session::Utf8ChunkDecoder;

#[test]
fn test_utf8_split_chinese_characters() {
    let mut decoder = Utf8ChunkDecoder::new();
    let text = "你好，世界！";
    let bytes = text.as_bytes();

    let chunk1 = &bytes[..4]; // "你" (3字节) + "好" 的第1个字节
    let chunk2 = &bytes[4..];  // "好" 的剩余2字节 + 其它

    let res1 = decoder.decode(chunk1);
    assert_eq!(res1, "你");

    let res2 = decoder.decode(chunk2);
    assert_eq!(res2, "好，世界！");
}

#[test]
fn test_utf8_invalid_bytes_replacement() {
    let mut decoder = Utf8ChunkDecoder::new();
    let raw = vec![0xFF, 0xFE, b'A', b'B'];
    let res = decoder.decode(&raw);
    assert!(res.contains("AB"));
}

use dosker_agent_proto::{crc32, crc32_update};

#[test]
fn crc32_of_empty_is_zero() {
    assert_eq!(crc32(b""), 0);
}

#[test]
fn crc32_of_123456789_is_cbf43926() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}

#[test]
fn crc32_update_matches_one_shot() {
    let data = b"The quick brown fox jumps over the lazy dog";
    let (first, second) = data.split_at(17);

    assert_eq!(crc32_update(crc32(first), second), crc32(data));
}

#[test]
fn crc32_detects_a_single_bit_change() {
    let original = b"dosker agent protocol";
    let mut changed = *original;
    changed[1] ^= 0x01;

    assert_ne!(crc32(original), crc32(&changed));
}

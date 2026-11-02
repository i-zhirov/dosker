//! IEEE CRC-32 with the reflected polynomial.

/// Reflected IEEE 802.3 polynomial.
pub const IEEE_POLY: u32 = 0xEDB8_8320;

/// Compute the IEEE CRC-32 of `bytes`.
pub fn crc32(bytes: &[u8]) -> u32 {
    crc32_update(0, bytes)
}

/// Continue an IEEE CRC-32 computation.
///
/// The `crc` argument is a value returned by [`crc32`] or a previous call to
/// this function, so split inputs produce the same value as one call.
pub fn crc32_update(crc: u32, bytes: &[u8]) -> u32 {
    let mut state = !crc;

    for &byte in bytes {
        state ^= u32::from(byte);
        for _ in 0..8 {
            if state & 1 == 0 {
                state >>= 1;
            } else {
                state = (state >> 1) ^ IEEE_POLY;
            }
        }
    }

    !state
}

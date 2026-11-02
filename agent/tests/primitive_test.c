#include <stdio.h>

#include "dosker.h"

static int failures;

static void check(int condition, const char *name)
{
    if (!condition) {
        printf("FAILED: %s\n", name);
        failures++;
    }
}

static void test_crc32_empty(void)
{
    u8 empty = 0;

    check(crc32(&empty, 0) == 0UL, "empty CRC");
}

static void test_crc32_check_vector(void)
{
    check(crc32((const u8 *)"123456789", 9) == 0xCBF43926UL,
          "123456789 CRC");
}

static void test_crc32_split_update(void)
{
    const u8 *data = (const u8 *)"123456789";
    u32 split;

    split = crc32_update(0xFFFFFFFFUL, data, 4);
    split = crc32_update(split, data + 4, 5);
    split ^= 0xFFFFFFFFUL;
    check(split == crc32(data, 9), "split update");
}

static void test_header_big_endian(void)
{
    u8 payload[3] = { 0xAA, 0xBB, 0xCC };
    u8 frame[MAX_FRAME_BYTES];
    u16 frame_len = 0;
    int rc;

    rc = frame_encode(FRAME_EXEC_DATA, 0x01020304UL, 0xA0B0C0D0UL,
                      payload, 3, frame, &frame_len);
    check(rc == 0, "big-endian header fields: encode");
    if (rc != 0) {
        return;
    }
    check(frame[0] == 'D' && frame[1] == 'K' &&
          frame[2] == 'R' && frame[3] == '1',
          "wire magic is DKR1");
    check(frame[4] == 1, "wire version is 1");
    check(frame[8] == 0x01 && frame[9] == 0x02 &&
          frame[10] == 0x03 && frame[11] == 0x04,
          "big-endian header fields: request_id");
    check(frame[12] == 0xA0 && frame[13] == 0xB0 &&
          frame[14] == 0xC0 && frame[15] == 0xD0,
          "big-endian header fields: sequence");
    check(frame[16] == 0x00 && frame[17] == 0x00 &&
          frame[18] == 0x00 && frame[19] == 0x03,
          "big-endian header fields: payload_length");
}

static void test_round_trip(void)
{
    u8 payload[5] = { 'h', 'e', 'l', 'l', 'o' };
    u8 frame[MAX_FRAME_BYTES];
    u16 frame_len = 0;
    struct frame_header header;
    int rc;

    rc = frame_encode(FRAME_EXEC_DATA, 42UL, 7UL, payload, 5, frame,
                      &frame_len);
    check(rc == 0, "round trip: encode");
    if (rc != 0) {
        return;
    }
    check(frame_len == (u16)(FRAME_HEADER_BYTES + 5 + 4),
          "round trip: frame length");
    rc = frame_decode_header(frame, &header);
    check(rc == 0, "round trip: decode");
    check(header.version == PROTOCOL_VERSION, "round trip: version");
    check(header.type == FRAME_EXEC_DATA, "round trip: type");
    check(header.flags == 0, "round trip: flags");
    check(header.reserved == 0, "round trip: reserved");
    check(header.request_id == 42UL, "round trip: request_id");
    check(header.sequence == 7UL, "round trip: sequence");
    check(header.payload_length == 5UL, "round trip: payload_length");
    check(frame[FRAME_HEADER_BYTES] == 'h' &&
          frame[FRAME_HEADER_BYTES + 1] == 'e' &&
          frame[FRAME_HEADER_BYTES + 2] == 'l' &&
          frame[FRAME_HEADER_BYTES + 3] == 'l' &&
          frame[FRAME_HEADER_BYTES + 4] == 'o',
          "round trip: payload");
}

static void test_crc_mismatch(void)
{
    u8 payload[2] = { 1, 2 };
    u8 frame[MAX_FRAME_BYTES];
    u16 frame_len = 0;
    u32 stored;
    u32 original_crc;
    int rc;

    rc = frame_encode(FRAME_PUT_DATA, 1UL, 2UL, payload, 2, frame,
                      &frame_len);
    check(rc == 0, "CRC mismatch: encode");
    if (rc != 0) {
        return;
    }
    stored = read_be32(frame + FRAME_HEADER_BYTES + 2);
    original_crc = frame_crc(frame, 2);
    check(original_crc == stored, "CRC mismatch: matching value");

    frame[8] ^= 0xFF;
    check(frame_crc(frame, 2) != original_crc,
          "header mutation changes CRC");
    check(frame_crc(frame, 2) != stored,
          "header mutation invalidates stored CRC");
    frame[8] ^= 0xFF;

    frame[FRAME_HEADER_BYTES] ^= 0xFF;
    check(frame_crc(frame, 2) != stored,
          "payload mutation invalidates stored CRC");
}

static u8 big_payload[MAX_PAYLOAD + 1];
static u8 big_frame[MAX_FRAME_BYTES];

static void test_payload_limits(void)
{
    u16 frame_len = 0;
    struct frame_header header;
    u16 i;
    int rc;

    for (i = 0; i < MAX_PAYLOAD; i++) {
        big_payload[i] = (u8)i;
    }
    rc = frame_encode(FRAME_EXEC_DATA, 1UL, 1UL, big_payload, MAX_PAYLOAD,
                      big_frame, &frame_len);
    check(rc == 0, "4096-byte payload accepted");
    if (rc != 0) {
        return;
    }
    check(frame_len == (u16)MAX_FRAME_BYTES, "4096-byte payload frame length");
    check(frame_decode_header(big_frame, &header) == 0,
          "4096-byte payload decoded");
    check(header.payload_length == (u32)MAX_PAYLOAD,
          "4096-byte payload length field");

    big_payload[MAX_PAYLOAD] = 0;
    check(frame_encode(FRAME_EXEC_DATA, 1UL, 1UL, big_payload,
                       MAX_PAYLOAD + 1, big_frame, &frame_len) == -1,
          "4097-byte payload rejected");
}

static void test_unknown_type_preserved(void)
{
    u8 frame[MAX_FRAME_BYTES];
    u16 frame_len = 0;
    struct frame_header header;
    int rc;

    rc = frame_encode(0x99, 0UL, 0UL, NULL, 0, frame, &frame_len);
    check(rc == 0, "unknown type: encode");
    if (rc != 0) {
        return;
    }
    check(frame_decode_header(frame, &header) == 0, "unknown type: decode");
    check(header.type == 0x99, "unknown type preserved");
}

static void test_header_rejections(void)
{
    u8 frame[MAX_FRAME_BYTES];
    u16 frame_len = 0;
    struct frame_header header;
    int rc;

    rc = frame_encode(FRAME_PING, 0UL, 0UL, NULL, 0, frame, &frame_len);
    check(rc == 0, "header rejections: encode");
    if (rc != 0) {
        return;
    }

    frame[0] = 'X';
    check(frame_decode_header(frame, &header) == -1, "bad magic rejected");
    frame[0] = (u8)FRAME_MAGIC[0];

    frame[4] = 2;
    check(frame_decode_header(frame, &header) == -1, "bad version rejected");
    frame[4] = PROTOCOL_VERSION;

    frame[6] = 1;
    check(frame_decode_header(frame, &header) == -1, "bad flags rejected");
    frame[6] = 0;

    frame[7] = 0xFF;
    check(frame_decode_header(frame, &header) == 0, "reserved is ignored");
    frame[7] = 0;

    write_be32(frame + 16, (u32)MAX_PAYLOAD + 1UL);
    check(frame_decode_header(frame, &header) == -1,
          "over-limit payload rejected");
}

int main(void)
{
    test_crc32_empty();
    test_crc32_check_vector();
    test_crc32_split_update();
    test_header_big_endian();
    test_round_trip();
    test_crc_mismatch();
    test_payload_limits();
    test_unknown_type_preserved();
    test_header_rejections();

    if (failures != 0) {
        printf("%d primitive test case(s) failed\n", failures);
        return 1;
    }
    printf("agent primitive tests passed\n");
    return 0;
}

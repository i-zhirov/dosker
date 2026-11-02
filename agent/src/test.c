#include "dosker.h"

#include <conio.h>
#include <stdio.h>
#include <string.h>

#define SELF_TEST_LOG "C:\\DOSKER\\TEST.LOG"
#define SELF_TEST_TMP "C:\\DOSKER\\TEST.TMP"
#define SELF_TEST_PORT 0x501

static u8 corrupt_frame[FRAME_HEADER_BYTES + 4];
static u16 fake_write_count;
static u16 fake_nak_count;
static u16 fake_error_count;
static u16 fake_corrupt_len;

static int fake_timeout_read(u8 *frame, u16 capacity, u16 *frame_len)
{
    (void)frame;
    (void)capacity;

    *frame_len = 0;
    return 0;
}

static int fake_count_write(const u8 *buf, u16 len)
{
    (void)buf;
    (void)len;

    fake_write_count++;
    return 1;
}

static int fake_corrupt_read(u8 *frame, u16 capacity, u16 *frame_len)
{
    if (capacity < fake_corrupt_len) {
        return -1;
    }

    memcpy(frame, corrupt_frame, fake_corrupt_len);
    *frame_len = fake_corrupt_len;
    return 1;
}

static int fake_classify_write(const u8 *buf, u16 len)
{
    if (len < FRAME_HEADER_BYTES) {
        return 1;
    }

    if (buf[5] == FRAME_NAK) {
        fake_nak_count++;
    } else if (buf[5] == FRAME_ERROR) {
        fake_error_count++;
    }

    return 1;
}

static int self_test_crc(void)
{
    if (crc32((const u8 *)"123456789", 9) != 0xCBF43926UL) {
        return 1;
    }

    if (crc32((const u8 *)"", 0) != 0UL) {
        return 2;
    }

    return 0;
}

static int self_test_frame(void)
{
    u8 frame[MAX_FRAME_BYTES];
    u8 payload[3];
    u16 frame_len;
    struct frame_header header;
    u32 expected_crc;

    payload[0] = (u8)'a';
    payload[1] = (u8)'b';
    payload[2] = (u8)'c';

    /* Case 3: encode and decode round trip. */
    if (frame_encode(FRAME_PING, 0x01020304UL, 0x05060708UL, payload, 3,
                     frame, &frame_len) != 0) {
        return 3;
    }
    if (frame_decode_header(frame, &header) != 0) {
        return 3;
    }
    if (header.type != FRAME_PING ||
        header.request_id != 0x01020304UL ||
        header.sequence != 0x05060708UL ||
        header.payload_length != 3UL) {
        return 3;
    }
    if (memcmp(frame + FRAME_HEADER_BYTES, payload, 3) != 0) {
        return 3;
    }
    expected_crc = frame_crc(frame, 3);
    if (read_be32(frame + FRAME_HEADER_BYTES + 3) != expected_crc) {
        return 3;
    }

    /* Case 4: header fields are big-endian. */
    if (frame_encode(FRAME_PING, 0x01020304UL, 0x05060708UL, payload, 3,
                     frame, &frame_len) != 0) {
        return 4;
    }
    if (frame[8] != 0x01 || frame[9] != 0x02 ||
        frame[10] != 0x03 || frame[11] != 0x04) {
        return 4;
    }
    if (frame[12] != 0x05 || frame[13] != 0x06 ||
        frame[14] != 0x07 || frame[15] != 0x08) {
        return 4;
    }
    if (frame[16] != 0x00 || frame[17] != 0x00 ||
        frame[18] != 0x00 || frame[19] != 0x03) {
        return 4;
    }

    /* Case 5: a CRC mismatch is detected with the header ids. */
    frame[frame_len - 1] = (u8)(frame[frame_len - 1] ^ 0xFF);
    if (frame_decode_header(frame, &header) != 0) {
        return 5;
    }
    if (header.request_id != 0x01020304UL ||
        header.sequence != 0x05060708UL) {
        return 5;
    }
    expected_crc = frame_crc(frame, (u16)header.payload_length);
    if (expected_crc == read_be32(frame + FRAME_HEADER_BYTES +
                                  header.payload_length)) {
        return 5;
    }

    /* Case 6: payload 4096 is accepted and 4097 is rejected. */
    if (frame_encode(FRAME_PING, 1UL, 1UL,
                     frame + FRAME_HEADER_BYTES, MAX_PAYLOAD,
                     frame, &frame_len) != 0) {
        return 6;
    }
    if (frame_len != MAX_FRAME_BYTES) {
        return 6;
    }
    if (frame_encode(FRAME_PING, 1UL, 1UL,
                     frame + FRAME_HEADER_BYTES,
                     (u16)(MAX_PAYLOAD + 1), frame, &frame_len) != -1) {
        return 6;
    }

    return 0;
}

static int self_test_config_and_paths(void)
{
    struct config cfg;
    char command[MAX_COMMAND_CHARS + 2];
    u16 index;

    /* Case 7: config defaults are COM2 and 115200. */
    config_defaults(&cfg);
    if (cfg.control_port != 2) {
        return 7;
    }
    if (cfg.baud != 115200UL) {
        return 7;
    }
    if (serial_base(cfg.control_port) != 0x2F8) {
        return 7;
    }

    /* Case 8: 8.3 path validation. */
    if (!path_is_valid("C:\\APP\\DATA.BIN")) {
        return 8;
    }
    if (path_is_valid("C:\\APP\\TOOLONGNAME.BIN")) {
        return 8;
    }

    /* Case 9: command length validation. */
    for (index = 0; index < MAX_COMMAND_CHARS; index++) {
        command[index] = 'A';
    }
    command[MAX_COMMAND_CHARS] = '\0';
    if (!command_is_valid(command)) {
        return 9;
    }

    command[MAX_COMMAND_CHARS] = 'A';
    command[MAX_COMMAND_CHARS + 1] = '\0';
    if (command_is_valid(command)) {
        return 9;
    }

    return 0;
}

static int self_test_file(void)
{
    FILE *file;
    const char *text;
    char readback[8];
    u16 count;

    text = "DOSKER";

    file = fopen(SELF_TEST_TMP, "wb");
    if (file == NULL) {
        return 10;
    }
    if ((u16)fwrite(text, 1, 6, file) != 6) {
        fclose(file);
        file_delete(SELF_TEST_TMP);
        return 10;
    }
    if (fclose(file) != 0) {
        file_delete(SELF_TEST_TMP);
        return 10;
    }

    file = fopen(SELF_TEST_TMP, "rb");
    if (file == NULL) {
        file_delete(SELF_TEST_TMP);
        return 10;
    }
    count = (u16)fread(readback, 1, 6, file);
    if (fclose(file) != 0) {
        file_delete(SELF_TEST_TMP);
        return 10;
    }
    if (count != 6 || memcmp(readback, text, 6) != 0) {
        file_delete(SELF_TEST_TMP);
        return 10;
    }

    if (file_delete(SELF_TEST_TMP) != 0) {
        return 10;
    }
    if (file_exists(SELF_TEST_TMP)) {
        return 10;
    }

    return 0;
}

static int self_test_command_not_found(void)
{
    const char *name_message;
    const char *filename_message;

    name_message = "Bad command or file name";
    filename_message = "Bad command or filename";

    if (!command_not_found_matches(name_message,
                                   (u32)strlen(name_message))) {
        return 11;
    }
    if (!command_not_found_matches(filename_message,
                                   (u32)strlen(filename_message))) {
        return 11;
    }
    if (command_not_found_matches("HELLO", 5UL)) {
        return 11;
    }

    return 0;
}

static int self_test_session_send(void)
{
    struct session session;
    struct session_io io;

    io.read_frame = fake_timeout_read;
    io.write_bytes = fake_count_write;

    fake_write_count = 0;
    session_init(&session, &io);
    session_begin_request(&session);

    if (session_send(&session, FRAME_PING, (const u8 *)0, 0) != -1) {
        return 12;
    }
    if (fake_write_count != (u16)(MAX_RETRIES + 1)) {
        return 12;
    }
    if (!session_is_closed(&session)) {
        return 12;
    }

    return 0;
}

static int self_test_session_recv(void)
{
    struct session session;
    struct session_io io;
    struct frame_header header;
    u8 payload[8];
    u16 payload_len;
    u16 frame_len;

    if (frame_encode(FRAME_PING, 0x11223344UL, 7UL, (const u8 *)0, 0,
                     corrupt_frame, &frame_len) != 0) {
        return 13;
    }
    corrupt_frame[frame_len - 1] =
        (u8)(corrupt_frame[frame_len - 1] ^ 0xFF);
    fake_corrupt_len = frame_len;

    io.read_frame = fake_corrupt_read;
    io.write_bytes = fake_classify_write;

    fake_nak_count = 0;
    fake_error_count = 0;
    session_init(&session, &io);

    if (session_recv(&session, &header, payload, &payload_len) != -1) {
        return 13;
    }
    if (fake_nak_count != 4) {
        return 13;
    }
    if (fake_error_count != 1) {
        return 13;
    }
    if (!session_is_closed(&session)) {
        return 13;
    }

    return 0;
}

int agent_self_test(void)
{
    int result;

    result = self_test_crc();
    if (result != 0) {
        return result;
    }

    result = self_test_frame();
    if (result != 0) {
        return result;
    }

    result = self_test_config_and_paths();
    if (result != 0) {
        return result;
    }

    result = self_test_file();
    if (result != 0) {
        return result;
    }

    result = self_test_command_not_found();
    if (result != 0) {
        return result;
    }

    result = self_test_session_send();
    if (result != 0) {
        return result;
    }

    result = self_test_session_recv();
    if (result != 0) {
        return result;
    }

    return 0;
}

void self_test_report(int result)
{
    FILE *file;
    char line[64];

    if (result == 0) {
        strcpy(line, "DOSKER SELF-TEST PASS");
    } else {
        sprintf(line, "DOSKER SELF-TEST FAIL %d", result);
    }

    file = fopen(SELF_TEST_LOG, "w");
    if (file != NULL) {
        fprintf(file, "%s\n", line);
        fclose(file);
    }

    printf("%s\n", line);

    outp(SELF_TEST_PORT, result == 0 ? 0 : 1);
}

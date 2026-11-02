#include "dosker.h"

#include <bios.h>
#include <conio.h>

#define TICKS_PER_SECOND 18UL
#define FRAME_TIMEOUT_TICKS 9UL
#define WRITE_TIMEOUT_TICKS 18UL

static u16 port_base;

static u32 tick_count(void)
{
    long ticks;

    ticks = 0L;
    _bios_timeofday(_TIME_GETCLOCK, &ticks);
    return (u32)ticks;
}

static int ticks_expired(u32 start, u32 limit)
{
    return (u32)(tick_count() - start) >= limit;
}

static int wait_for_data(u32 start, u32 limit)
{
    while ((inp(port_base + 5) & 0x01) == 0) {
        if (ticks_expired(start, limit)) {
            return 0;
        }
    }
    return 1;
}

static int read_exact_until(u8 *buffer, u16 len, u32 start, u32 limit)
{
    u16 i;

    for (i = 0; i < len; i++) {
        if (!wait_for_data(start, limit)) {
            return 0;
        }
        buffer[i] = (u8)inp(port_base);
    }

    return 1;
}

int serial_open(const struct config *cfg)
{
    u16 divisor;
    u16 base;

    base = serial_base(cfg->control_port);
    if (base == 0) {
        return 0;
    }
    if (baud_divisor(cfg->baud, &divisor) != 0) {
        return 0;
    }

    port_base = base;

    outp(port_base + 1, 0x00);
    outp(port_base + 3, 0x80);
    outp(port_base + 0, (int)(divisor & 0xFF));
    outp(port_base + 1, (int)(divisor >> 8));
    outp(port_base + 3, 0x03);
    outp(port_base + 2, 0xC7);
    outp(port_base + 4, 0x0B);

    return 1;
}

int serial_data_ready(void)
{
    return (inp(port_base + 5) & 0x01) != 0;
}

int serial_read_byte(u8 *value)
{
    if (!serial_data_ready()) {
        return 0;
    }

    *value = (u8)inp(port_base);
    return 1;
}

int serial_write_byte(u8 value)
{
    u32 start;

    start = tick_count();
    while ((inp(port_base + 5) & 0x20) == 0) {
        if (ticks_expired(start, WRITE_TIMEOUT_TICKS)) {
            return 0;
        }
    }

    outp(port_base, (int)value);
    return 1;
}

int serial_write_all(const u8 *buffer, u16 len)
{
    u16 i;

    for (i = 0; i < len; i++) {
        if (!serial_write_byte(buffer[i])) {
            return 0;
        }
    }

    return 1;
}

int serial_read_exact(u8 *buffer, u16 len)
{
    u32 start;

    if (len == 0) {
        return 1;
    }

    start = tick_count();
    if (!wait_for_data(start, FRAME_TIMEOUT_TICKS)) {
        return 0;
    }

    return read_exact_until(buffer, len, start, FRAME_TIMEOUT_TICKS);
}

int serial_read_frame(u8 *frame, u16 capacity, u16 *frame_len)
{
    struct frame_header header;
    u32 start;
    u32 total;
    u16 matched;
    u8 value;

    start = tick_count();
    matched = 0;
    while (matched < 4) {
        if (!wait_for_data(start, FRAME_TIMEOUT_TICKS)) {
            return 0;
        }

        value = (u8)inp(port_base);
        if (value == (u8)FRAME_MAGIC[matched]) {
            matched++;
        } else if (value == (u8)FRAME_MAGIC[0]) {
            matched = 1;
        } else {
            matched = 0;
        }
    }

    frame[0] = (u8)FRAME_MAGIC[0];
    frame[1] = (u8)FRAME_MAGIC[1];
    frame[2] = (u8)FRAME_MAGIC[2];
    frame[3] = (u8)FRAME_MAGIC[3];

    start = tick_count();
    if (!read_exact_until(frame + 4, FRAME_HEADER_BYTES - 4, start,
                          FRAME_TIMEOUT_TICKS)) {
        return -1;
    }
    if (frame_decode_header(frame, &header) != 0) {
        return -1;
    }

    total = FRAME_HEADER_BYTES + header.payload_length + 4;
    if (total > (u32)capacity || total > (u32)MAX_FRAME_BYTES) {
        return -1;
    }

    if (!read_exact_until(frame + FRAME_HEADER_BYTES,
                          (u16)(total - FRAME_HEADER_BYTES), start,
                          FRAME_TIMEOUT_TICKS)) {
        return -1;
    }

    *frame_len = (u16)total;
    return 1;
}

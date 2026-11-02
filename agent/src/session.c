#include "dosker.h"

#include <stddef.h>
#include <string.h>

#define MAX_CORRUPT_FRAMES 4

static const struct session_io *session_io;
static u16 consecutive_corrupt;
static u8 control_frame[MAX_FRAME_BYTES];
static u8 out_frame[MAX_FRAME_BYTES];
static u8 in_frame[MAX_FRAME_BYTES];
static u8 raw_frame[MAX_FRAME_BYTES];
static char error_json[128];

static int frame_is_valid(const u8 *frame, u16 frame_len,
                          struct frame_header *header)
{
    u32 expected;
    u32 actual;

    if (frame_len < FRAME_HEADER_BYTES + 4) {
        return 0;
    }
    if (frame_decode_header(frame, header) != 0) {
        return 0;
    }
    if ((u32)FRAME_HEADER_BYTES + header->payload_length + 4 !=
        (u32)frame_len) {
        return 0;
    }

    expected = frame_crc(frame, (u16)header->payload_length);
    actual = read_be32(frame + FRAME_HEADER_BYTES +
                       header->payload_length);

    return expected == actual;
}

static int write_control(u8 type, u32 request_id, u32 sequence)
{
    u16 frame_len;

    if (session_io == NULL) {
        return -1;
    }
    if (frame_encode(type, request_id, sequence, (const u8 *)0, 0,
                     control_frame, &frame_len) != 0) {
        return -1;
    }

    return session_io->write_bytes(control_frame, frame_len) ? 0 : -1;
}

static int send_crc_error(struct session *s, u32 request_id)
{
    u16 length;

    if (!json_write_error(error_json, (u16)sizeof(error_json), "CRC",
                          "frame failed CRC after retries", request_id)) {
        return -1;
    }

    length = (u16)strlen(error_json);
    return session_write_frame(s, FRAME_ERROR, request_id, 0,
                               (const u8 *)error_json, length);
}

int session_init(struct session *s, const struct session_io *io)
{
    session_io = io;
    consecutive_corrupt = 0;

    s->request_id = 0;
    s->sequence = 0;
    s->closed = 0;
    s->session_id[0] = '\0';

    return 0;
}

u32 session_begin_request(struct session *s)
{
    s->request_id = s->request_id + 1UL;
    s->sequence = 0;
    return s->request_id;
}

void session_use_request(struct session *s, u32 request_id)
{
    s->request_id = request_id;
    s->sequence = 0;
}

int session_ack(struct session *s, u32 request_id, u32 sequence)
{
    if (s->closed) {
        return -1;
    }

    return write_control(FRAME_ACK, request_id, sequence);
}

int session_write_frame(struct session *s, u8 type, u32 request_id,
                        u32 sequence, const u8 *payload, u16 len)
{
    u16 frame_len;

    if (s->closed || session_io == NULL) {
        return -1;
    }
    if (frame_encode(type, request_id, sequence, payload, len, raw_frame,
                     &frame_len) != 0) {
        return -1;
    }

    return session_io->write_bytes(raw_frame, frame_len) ? 0 : -1;
}

int session_send(struct session *s, u8 type, const u8 *payload, u16 len)
{
    u16 frame_len;
    u16 response_len;
    struct frame_header header;
    int attempt;
    int rc;

    if (s->closed || session_io == NULL) {
        return -1;
    }
    if (frame_encode(type, s->request_id, s->sequence, payload, len,
                     out_frame, &frame_len) != 0) {
        return -1;
    }

    for (attempt = 0; attempt <= MAX_RETRIES; attempt++) {
        if (!session_io->write_bytes(out_frame, frame_len)) {
            s->closed = 1;
            return -1;
        }

        rc = session_io->read_frame(in_frame, MAX_FRAME_BYTES, &response_len);
        if (rc == 1 && frame_is_valid(in_frame, response_len, &header)) {
            if (header.type == FRAME_ACK) {
                if (header.request_id == s->request_id &&
                    header.sequence == s->sequence) {
                    s->sequence = s->sequence + 1UL;
                    return 0;
                }
            } else if (header.type == FRAME_NAK) {
                if (header.request_id == s->request_id &&
                    header.sequence == s->sequence) {
                    continue;
                }
            }
        }
    }

    s->closed = 1;
    return -1;
}

int session_recv(struct session *s, struct frame_header *header, u8 *payload,
                 u16 *payload_len)
{
    u16 frame_len;
    u32 request_id;
    u32 sequence;
    int rc;
    u16 i;

    if (s->closed || session_io == NULL) {
        return -1;
    }

    for (;;) {
        rc = session_io->read_frame(in_frame, MAX_FRAME_BYTES, &frame_len);
        if (rc == 0) {
            return 0;
        }
        if (rc < 0) {
            s->closed = 1;
            return -1;
        }

        if (!frame_is_valid(in_frame, frame_len, header)) {
            request_id = 0;
            sequence = 0;
            if (frame_decode_header(in_frame, header) == 0) {
                request_id = header->request_id;
                sequence = header->sequence;
            }
            write_control(FRAME_NAK, request_id, sequence);

            consecutive_corrupt++;
            if (consecutive_corrupt >= MAX_CORRUPT_FRAMES) {
                send_crc_error(s, request_id);
                s->closed = 1;
                return -1;
            }
            continue;
        }

        consecutive_corrupt = 0;

        if (header->type == FRAME_ACK || header->type == FRAME_NAK) {
            continue;
        }

        for (i = 0; i < (u16)header->payload_length; i++) {
            payload[i] = in_frame[FRAME_HEADER_BYTES + i];
        }
        *payload_len = (u16)header->payload_length;

        return 1;
    }
}

int session_is_closed(const struct session *s)
{
    return s->closed;
}

void session_close(struct session *s)
{
    s->closed = 1;
}

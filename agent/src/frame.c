#include "dosker.h"

void write_be32(u8 *out, u32 value)
{
    out[0] = (u8)(value >> 24);
    out[1] = (u8)(value >> 16);
    out[2] = (u8)(value >> 8);
    out[3] = (u8)value;
}

u32 read_be32(const u8 *in)
{
    return ((u32)in[0] << 24) |
           ((u32)in[1] << 16) |
           ((u32)in[2] << 8) |
           (u32)in[3];
}

int frame_encode(u8 type, u32 request_id, u32 sequence, const u8 *payload,
                 u16 len, u8 *out, u16 *out_len)
{
    u16 i;
    u32 crc;

    if (len > MAX_PAYLOAD) {
        return -1;
    }

    out[0] = (u8)FRAME_MAGIC[0];
    out[1] = (u8)FRAME_MAGIC[1];
    out[2] = (u8)FRAME_MAGIC[2];
    out[3] = (u8)FRAME_MAGIC[3];
    out[4] = (u8)PROTOCOL_VERSION;
    out[5] = type;
    out[6] = 0;
    out[7] = 0;
    write_be32(out + 8, request_id);
    write_be32(out + 12, sequence);
    write_be32(out + 16, (u32)len);

    for (i = 0; i < len; i++) {
        out[FRAME_HEADER_BYTES + i] = payload[i];
    }

    crc = frame_crc(out, len);
    write_be32(out + FRAME_HEADER_BYTES + len, crc);
    *out_len = (u16)(FRAME_HEADER_BYTES + len + 4);

    return 0;
}

int frame_decode_header(const u8 *in, struct frame_header *out)
{
    u32 payload_length;

    if ((u8)in[0] != (u8)FRAME_MAGIC[0] ||
        (u8)in[1] != (u8)FRAME_MAGIC[1] ||
        (u8)in[2] != (u8)FRAME_MAGIC[2] ||
        (u8)in[3] != (u8)FRAME_MAGIC[3]) {
        return -1;
    }
    if (in[4] != (u8)PROTOCOL_VERSION) {
        return -1;
    }
    if (in[6] != 0) {
        return -1;
    }

    payload_length = read_be32(in + 16);
    if (payload_length > (u32)MAX_PAYLOAD) {
        return -1;
    }

    out->version = in[4];
    out->type = in[5];
    out->flags = in[6];
    out->reserved = in[7];
    out->request_id = read_be32(in + 8);
    out->sequence = read_be32(in + 12);
    out->payload_length = payload_length;

    return 0;
}

u32 frame_crc(const u8 *frame, u16 len)
{
    return crc32(frame, (u16)(FRAME_HEADER_BYTES + len));
}

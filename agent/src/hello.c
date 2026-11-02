#include "dosker.h"

#include <bios.h>
#include <string.h>

#define TICKS_PER_SECOND 18UL
#define HELLO_INTERVAL_TICKS TICKS_PER_SECOND
#define HANDSHAKE_LIMIT_TICKS (10UL * TICKS_PER_SECOND)
#define HELLO_JSON_CHARS 256

static u32 tick_count(void)
{
    long ticks;

    ticks = 0L;
    _bios_timeofday(_TIME_GETCLOCK, &ticks);
    return (u32)ticks;
}

static u16 memory_kb(void)
{
    return (u16)_bios_memsize();
}

int send_hello(struct session *s, const struct config *cfg)
{
    char json[HELLO_JSON_CHARS];
    u32 request_id;
    u16 length;

    request_id = session_begin_request(s);

    if (!json_write_hello(json, sizeof(json), cfg, memory_kb())) {
        return -1;
    }

    length = (u16)strlen(json);
    return session_write_frame(s, FRAME_HELLO, request_id, 0,
                               (const u8 *)json, length);
}

int agent_handshake(struct session *s, const struct config *cfg)
{
    struct frame_header header;
    static u8 payload[MAX_PAYLOAD + 1];
    u16 payload_len;
    u32 start;
    u32 next_send;
    int sent;
    int rc;

    start = tick_count();
    next_send = start;
    sent = 0;

    while ((u32)(tick_count() - start) < HANDSHAKE_LIMIT_TICKS) {
        u32 now;

        rc = session_recv(s, &header, payload, &payload_len);
        if (rc == 1) {
            payload[payload_len] = '\0';

            if (header.type == FRAME_HELLO_ACK) {
                session_ack(s, header.request_id, header.sequence);
                json_parse_session_id((const char *)payload,
                                      (char *)s->session_id,
                                      (u16)sizeof(s->session_id));
                return 1;
            }

            if (header.type == FRAME_PING) {
                session_ack(s, header.request_id, header.sequence);
                send_hello(s, cfg);
                sent = 1;
                next_send = tick_count();
            }

            continue;
        }
        if (rc < 0) {
            return 0;
        }

        now = tick_count();
        if (!sent || (u32)(now - next_send) >= HELLO_INTERVAL_TICKS) {
            send_hello(s, cfg);
            sent = 1;
            next_send = now;
        }
    }

    return 0;
}

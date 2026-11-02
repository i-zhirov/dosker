#include "dosker.h"

#include <string.h>

#define JSON_CHARS 256

static int agent_busy;

static char transfer_temp[128];
static char transfer_target[128];
static u32 transfer_size;
static u32 transfer_remaining;
static int transfer_active;
static u32 put_request_id;
static int put_request_seen;
static u32 put_data_request_id;
static u32 put_data_sequence;
static int put_data_seen;

static const char *transfer_code(int result)
{
    switch (result) {
    case TRANSFER_BAD_PATH:
        return "BAD_PATH";
    case TRANSFER_NO_FILE:
        return "NO_FILE";
    case TRANSFER_NO_SPACE:
        return "NO_SPACE";
    case TRANSFER_TOO_LARGE:
        return "TOO_LARGE";
    default:
        return "INTERNAL";
    }
}

static const char *transfer_message(int result)
{
    switch (result) {
    case TRANSFER_BAD_PATH:
        return "path is invalid or not 8.3";
    case TRANSFER_NO_FILE:
        return "path does not exist";
    case TRANSFER_NO_SPACE:
        return "not enough free space";
    case TRANSFER_TOO_LARGE:
        return "file exceeds the transfer limit";
    default:
        return "agent internal error";
    }
}

static void transfer_reset(void)
{
    transfer_temp[0] = '\0';
    transfer_target[0] = '\0';
    transfer_size = 0;
    transfer_remaining = 0;
    transfer_active = 0;
    put_request_id = 0;
    put_request_seen = 0;
    put_data_request_id = 0;
    put_data_sequence = 0;
    put_data_seen = 0;
}

static int send_put_end(struct session *s, u32 bytes)
{
    char json[JSON_CHARS];
    u16 length;

    if (!json_write_put_end(json, (u16)sizeof(json), 1, bytes)) {
        return -1;
    }

    length = (u16)strlen(json);
    return session_send(s, FRAME_PUT_END, (const u8 *)json, length);
}

int send_error(struct session *s, const char *code, const char *message,
               u32 request_id)
{
    char json[JSON_CHARS];
    u16 length;

    session_use_request(s, request_id);

    if (!json_write_error(json, sizeof(json), code, message, request_id)) {
        return -1;
    }

    length = (u16)strlen(json);
    return session_send(s, FRAME_ERROR, (const u8 *)json, length);
}

int agent_loop(struct session *s, const struct config *cfg)
{
    struct frame_header header;
    static u8 payload[MAX_PAYLOAD + 1];
    u16 payload_len;
    char json[JSON_CHARS];
    u16 length;
    int rc;

    for (;;) {
        rc = session_recv(s, &header, payload, &payload_len);
        if (rc == 0) {
            continue;
        }
        if (rc < 0) {
            return 1;
        }

        payload[payload_len] = '\0';

        switch (header.type) {
        case FRAME_HELLO_ACK:
            session_ack(s, header.request_id, header.sequence);
            json_parse_session_id((const char *)payload,
                                  (char *)s->session_id,
                                  (u16)sizeof(s->session_id));
            break;

        case FRAME_PING:
            session_ack(s, header.request_id, header.sequence);
            if (s->session_id[0] == '\0') {
                send_hello(s, cfg);
            } else {
                session_use_request(s, header.request_id);
                if (json_write_pong(json, sizeof(json), agent_busy)) {
                    length = (u16)strlen(json);
                    session_send(s, FRAME_PONG, (const u8 *)json, length);
                }
            }
            break;

        case FRAME_EXEC_REQ: {
            struct exec_request request;
            struct exec_result result;
            int parsed;

            if (agent_busy) {
                send_error(s, "BUSY", "a command is already running",
                           header.request_id);
                break;
            }

            parsed = json_parse_exec_request((const char *)payload, &request);
            if (parsed < 0) {
                send_error(s, "TOO_LONG",
                           "command or path exceeds the DOS limit",
                           header.request_id);
                break;
            }
            if (parsed == 0) {
                send_error(s, "INTERNAL", "agent internal error",
                           header.request_id);
                break;
            }

            session_ack(s, header.request_id, header.sequence);
            session_use_request(s, header.request_id);
            agent_busy = 1;
            exec_run(s, &request, &result);
            agent_busy = 0;
            break;
        }

        case FRAME_PUT_REQ: {
            char path[128];
            u32 size;
            int overwrite;
            int parsed;
            int result;

            if (transfer_active) {
                if (put_request_seen &&
                    header.request_id == put_request_id) {
                    session_ack(s, header.request_id, header.sequence);
                    break;
                }
                put_abort(transfer_temp);
                transfer_reset();
            }

            parsed = json_parse_put_request((const char *)payload, path,
                                            (u16)sizeof(path), &size,
                                            &overwrite);
            if (parsed < 0) {
                send_error(s, "TOO_LONG",
                           "command or path exceeds the DOS limit",
                           header.request_id);
                break;
            }
            if (parsed == 0) {
                send_error(s, "INTERNAL", "agent internal error",
                           header.request_id);
                break;
            }

            result = put_begin(path, size, overwrite, transfer_temp,
                               (u16)sizeof(transfer_temp));
            if (result != 0) {
                send_error(s, transfer_code(result),
                           transfer_message(result), header.request_id);
                break;
            }

            strcpy(transfer_target, path);
            transfer_size = size;
            transfer_remaining = size;
            transfer_active = 1;
            put_request_seen = 1;
            put_request_id = header.request_id;
            put_data_seen = 0;
            put_data_request_id = 0;
            put_data_sequence = 0;

            session_ack(s, header.request_id, header.sequence);

            if (size == 0) {
                if (put_finish(transfer_temp, transfer_target) != 0) {
                    put_abort(transfer_temp);
                    transfer_reset();
                    send_error(s, "INTERNAL", "agent internal error",
                               header.request_id);
                    break;
                }
                session_use_request(s, header.request_id);
                send_put_end(s, transfer_size);
                transfer_reset();
            }
            break;
        }

        case FRAME_PUT_DATA:
            if (!transfer_active) {
                send_error(s, "INTERNAL", "agent internal error",
                           header.request_id);
                break;
            }

            if (put_data_seen &&
                header.request_id == put_data_request_id &&
                header.sequence == put_data_sequence) {
                session_ack(s, header.request_id, header.sequence);
                break;
            }

            if ((u32)payload_len > transfer_remaining) {
                put_abort(transfer_temp);
                transfer_reset();
                send_error(s, "TOO_LARGE", "file exceeds the transfer limit",
                           header.request_id);
                break;
            }

            if (!put_chunk(payload, payload_len)) {
                put_abort(transfer_temp);
                transfer_reset();
                send_error(s, "INTERNAL", "agent internal error",
                           header.request_id);
                break;
            }

            transfer_remaining -= (u32)payload_len;
            put_data_seen = 1;
            put_data_request_id = header.request_id;
            put_data_sequence = header.sequence;
            session_ack(s, header.request_id, header.sequence);

            if (transfer_remaining == 0) {
                if (put_finish(transfer_temp, transfer_target) != 0) {
                    put_abort(transfer_temp);
                    transfer_reset();
                    send_error(s, "INTERNAL", "agent internal error",
                               header.request_id);
                    break;
                }

                session_use_request(s, header.request_id);
                send_put_end(s, transfer_size);
                transfer_reset();
            }
            break;

        case FRAME_GET_REQ: {
            char path[128];
            u32 size;
            int parsed;
            int result;

            parsed = json_parse_path_request((const char *)payload, path,
                                              (u16)sizeof(path));
            if (parsed < 0) {
                send_error(s, "TOO_LONG",
                           "command or path exceeds the DOS limit",
                           header.request_id);
                break;
            }
            if (parsed == 0) {
                send_error(s, "INTERNAL", "agent internal error",
                           header.request_id);
                break;
            }

            result = get_begin(path, &size);
            if (result != 0) {
                send_error(s, transfer_code(result),
                           transfer_message(result), header.request_id);
                break;
            }

            session_ack(s, header.request_id, header.sequence);
            session_use_request(s, header.request_id);
            get_stream(s, path, size);
            break;
        }

        case FRAME_STAT_REQ: {
            char path[128];
            struct stat_result result;
            int parsed;

            parsed = json_parse_path_request((const char *)payload, path,
                                              (u16)sizeof(path));
            if (parsed < 0) {
                send_error(s, "TOO_LONG",
                           "command or path exceeds the DOS limit",
                           header.request_id);
                break;
            }
            if (parsed == 0 || !path_is_valid(path)) {
                send_error(s, "BAD_PATH", "path is invalid or not 8.3",
                           header.request_id);
                break;
            }

            stat_path(path, &result);
            session_ack(s, header.request_id, header.sequence);
            session_use_request(s, header.request_id);

            if (!json_write_stat(json, (u16)sizeof(json), &result)) {
                send_error(s, "INTERNAL", "agent internal error",
                           header.request_id);
                break;
            }

            length = (u16)strlen(json);
            session_send(s, FRAME_STAT_RESP, (const u8 *)json, length);
            break;
        }

        case FRAME_SHUTDOWN: {
            u32 grace_ms;

            if (!json_parse_shutdown_request((const char *)payload,
                                             &grace_ms)) {
                send_error(s, "INTERNAL", "agent internal error",
                           header.request_id);
                break;
            }
            (void)grace_ms;

            session_ack(s, header.request_id, header.sequence);

            if (transfer_active) {
                put_abort(transfer_temp);
                transfer_reset();
            }

            session_use_request(s, header.request_id);

            if (!json_write_shutdown_ack(json, sizeof(json), 1)) {
                send_error(s, "INTERNAL", "agent internal error",
                           header.request_id);
                break;
            }

            length = (u16)strlen(json);
            session_send(s, FRAME_SHUTDOWN_ACK, (const u8 *)json, length);
            return 0;
        }

        default:
            send_error(s, "UNSUPPORTED", "unknown frame type",
                       header.request_id);
            break;
        }
    }
}

static void discard_pending_frames(void)
{
    static u8 frame[MAX_FRAME_BYTES];
    u16 length;

    for (;;) {
        if (serial_read_frame(frame, (u16)sizeof(frame), &length) == 0) {
            break;
        }
    }
}

static void prime_free_space(void)
{
    /* The first free-space query scans the FAT and is slow. Prime it
       before the handshake so a transfer request answers promptly. */
    (void)free_space("C:\\");
    discard_pending_frames();
}

int main(int argc, char **argv)
{
    struct config cfg;
    struct session session;
    struct session_io io;

    if (argc > 1 &&
        (strcmp(argv[1], "/TEST") == 0 || strcmp(argv[1], "/test") == 0)) {
        int result;

        result = agent_self_test();
        self_test_report(result);
        return result == 0 ? 0 : 1;
    }

    config_defaults(&cfg);
    config_load(&cfg);

    if (!serial_open(&cfg)) {
        return 1;
    }

    prime_free_space();

    io.read_frame = serial_read_frame;
    io.write_bytes = serial_write_all;

    session_init(&session, &io);
    agent_handshake(&session, &cfg);

    return agent_loop(&session, &cfg) == 0 ? 0 : 1;
}

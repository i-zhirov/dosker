#ifndef DOSKER_H
#define DOSKER_H

#ifdef __WATCOMC__
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned long u32;
#else
#include <stdint.h>
typedef uint8_t u8;
typedef uint16_t u16;
typedef uint32_t u32;
#endif

#define FRAME_MAGIC "DKR1"
#define PROTOCOL_VERSION 1
#define FRAME_HEADER_BYTES 20
#define MAX_PAYLOAD 4096
#define MAX_FRAME_BYTES (FRAME_HEADER_BYTES + MAX_PAYLOAD + 4)
#define MAX_OUTPUT_BYTES 1048576UL
#define MAX_COMMAND_CHARS 127
#define MAX_TRANSFER_BYTES 16777216UL
#define DEFAULT_TIMEOUT_MS 500
#define MAX_RETRIES 3
#define AGENT_VERSION "1.0.0"

#define FRAME_HELLO 0x01
#define FRAME_HELLO_ACK 0x02
#define FRAME_ACK 0x03
#define FRAME_NAK 0x04
#define FRAME_EXEC_REQ 0x10
#define FRAME_EXEC_DATA 0x11
#define FRAME_EXEC_END 0x12
#define FRAME_PUT_REQ 0x20
#define FRAME_PUT_DATA 0x21
#define FRAME_PUT_END 0x22
#define FRAME_GET_REQ 0x23
#define FRAME_GET_DATA 0x24
#define FRAME_GET_END 0x25
#define FRAME_STAT_REQ 0x26
#define FRAME_STAT_RESP 0x27
#define FRAME_PING 0x30
#define FRAME_PONG 0x31
#define FRAME_SHUTDOWN 0x40
#define FRAME_SHUTDOWN_ACK 0x41
#define FRAME_ERROR 0x7F

struct frame_header {
    u8 version;
    u8 type;
    u8 flags;
    u8 reserved;
    u32 request_id;
    u32 sequence;
    u32 payload_length;
};

struct config {
    u8 control_port;
    u32 baud;
};

struct session {
    u32 request_id;
    u32 sequence;
    int closed;
    u8 session_id[7];
};

struct session_io {
    int (*read_frame)(u8 *frame, u16 capacity, u16 *frame_len);
    int (*write_bytes)(const u8 *buf, u16 len);
};

struct env_pair {
    char key[32];
    char value[128];
};

struct exec_request {
    char command[128];
    char workdir[128];
    struct env_pair env[8];
    u16 env_count;
    u32 max_output_bytes;
};

struct exec_result {
    int exit_code;
    u32 output_bytes;
    int truncated;
    u32 duration_ms;
    int command_not_found;
};

struct stat_result {
    int exists;
    u32 size;
    int directory;
    u8 attributes;
    char timestamp[21];
};

#define TRANSFER_BAD_PATH 1
#define TRANSFER_NO_FILE 2
#define TRANSFER_NO_SPACE 3
#define TRANSFER_TOO_LARGE 4
#define TRANSFER_INTERNAL 5

u32 crc32(const u8 *data, u16 len);
u32 crc32_update(u32 crc, const u8 *data, u16 len);
void write_be32(u8 *out, u32 value);
u32 read_be32(const u8 *in);
int frame_encode(u8 type, u32 request_id, u32 sequence, const u8 *payload,
                 u16 len, u8 *out, u16 *out_len);
int frame_decode_header(const u8 *in, struct frame_header *out);
u32 frame_crc(const u8 *frame, u16 len);

void config_defaults(struct config *cfg);
int config_load(struct config *cfg);
u16 serial_base(u8 port);
int baud_divisor(u32 baud, u16 *divisor);

int serial_open(const struct config *cfg);
int serial_read_byte(u8 *value);
int serial_write_byte(u8 value);
int serial_read_exact(u8 *buffer, u16 len);
int serial_write_all(const u8 *buffer, u16 len);
int serial_data_ready(void);
int serial_read_frame(u8 *frame, u16 capacity, u16 *frame_len);

int session_init(struct session *s, const struct session_io *io);
u32 session_begin_request(struct session *s);
void session_use_request(struct session *s, u32 request_id);
int session_ack(struct session *s, u32 request_id, u32 sequence);
int session_write_frame(struct session *s, u8 type, u32 request_id,
                        u32 sequence, const u8 *payload, u16 len);
int session_send(struct session *s, u8 type, const u8 *payload, u16 len);
int session_recv(struct session *s, struct frame_header *header, u8 *payload,
                 u16 *payload_len);
int session_is_closed(const struct session *s);
void session_close(struct session *s);

int json_escape(const char *input, char *output, u16 output_len);
int json_get_string(const char *json, const char *key, char *output,
                    u16 output_len);
int json_get_u32(const char *json, const char *key, u32 *output);
int json_get_bool(const char *json, const char *key, int *output);
int json_write_hello(char *output, u16 output_len, const struct config *cfg,
                     u16 memory_kb);
int json_write_pong(char *output, u16 output_len, int busy);
int json_write_error(char *output, u16 output_len, const char *code,
                     const char *message, u32 request_id);
int json_parse_session_id(const char *json, char *output, u16 output_len);
int json_parse_env(const char *json, struct env_pair *pairs, u16 max_pairs,
                   u16 *count);
int json_parse_exec_request(const char *json, struct exec_request *request);
int json_write_exec_end(char *output, u16 output_len,
                        const struct exec_result *result);
int json_parse_put_request(const char *json, char *path, u16 path_len,
                           u32 *size, int *overwrite);
int json_parse_path_request(const char *json, char *path, u16 path_len);
int json_write_put_end(char *output, u16 output_len, int ok, u32 bytes);
int json_write_get_end(char *output, u16 output_len, int ok, u32 size);
int json_write_stat(char *output, u16 output_len,
                    const struct stat_result *result);
int json_parse_shutdown_request(const char *json, u32 *grace_ms);
int json_write_shutdown_ack(char *output, u16 output_len, int ready);

int path_is_valid(const char *path);
int command_is_valid(const char *command);
int path_parent(const char *path, char *directory, u16 directory_len);
int path_leaf(const char *path, char *leaf, u16 leaf_len);
int path_join(char *output, u16 output_len, const char *directory,
              const char *name);

int exec_run(struct session *s, const struct exec_request *request,
             struct exec_result *result);
int command_not_found_matches(const char *output, u32 len);

int file_exists(const char *path);
int file_size(const char *path, u32 *size);
int file_attributes(const char *path, u8 *attributes);
int file_timestamp(const char *path, char *output, u16 output_len);
int file_delete(const char *path);
int file_rename(const char *from, const char *to);
u32 free_space(const char *path);
int temp_path_for(const char *target, char *output, u16 output_len);

int stat_path(const char *path, struct stat_result *result);

int put_begin(const char *path, u32 size, int overwrite, char *temp_path,
              u16 temp_len);
int put_chunk(const u8 *data, u16 len);
int put_finish(const char *temp_path, const char *target);
void put_abort(const char *temp_path);
int get_begin(const char *path, u32 *size);
int get_stream(struct session *s, const char *path, u32 size);

int agent_handshake(struct session *s, const struct config *cfg);
int send_hello(struct session *s, const struct config *cfg);

int agent_loop(struct session *s, const struct config *cfg);
int send_error(struct session *s, const char *code, const char *message,
               u32 request_id);

int agent_self_test(void);
void self_test_report(int result);

#endif

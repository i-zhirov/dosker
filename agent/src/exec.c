#include "dosker.h"

#include <bios.h>
#include <dos.h>
#include <process.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define BATCH_PATH "C:\\DOSKER\\RUN.BAT"
#define OUTPUT_PATH "C:\\DOSKER\\OUT.TMP"
#define ERROR_PATH "C:\\DOSKER\\ERR.TMP"
#define DEFAULT_COMSPEC "C:\\COMMAND.COM"
#define FRAME_CHUNK MAX_PAYLOAD
#define MATCH_OVERLAP 32

static u8 output_buffer[FRAME_CHUNK];
static char scan_buffer[FRAME_CHUNK + MATCH_OVERLAP];
static char match_tail[MATCH_OVERLAP];

static const char *comspec_path(void)
{
    const char *value;

    value = getenv("COMSPEC");
    if (value == NULL || *value == '\0') {
        return DEFAULT_COMSPEC;
    }
    return value;
}

static int write_batch(const struct exec_request *request)
{
    FILE *file;
    u16 index;
    int ok;

    file = fopen(BATCH_PATH, "wb");
    if (file == NULL) {
        return 0;
    }

    ok = 1;
    if (fprintf(file, "@ECHO OFF\r\n") < 0) {
        ok = 0;
    }
    for (index = 0; ok && index < request->env_count; index++) {
        if (fprintf(file, "SET %s=%s\r\n", request->env[index].key,
                    request->env[index].value) < 0) {
            ok = 0;
        }
    }
    if (ok && request->workdir[0] != '\0') {
        if (fprintf(file, "CD %s\r\n", request->workdir) < 0) {
            ok = 0;
        }
    }
    if (ok) {
        if (fprintf(file, "%s > %s\r\n", request->command,
                    OUTPUT_PATH) < 0) {
            ok = 0;
        }
    }
    if (fclose(file) != 0) {
        ok = 0;
    }

    return ok;
}

static long bios_ticks(void)
{
    long ticks;

    ticks = 0L;
    _bios_timeofday(_TIME_GETCLOCK, &ticks);
    return ticks;
}

static u32 ticks_to_ms(u32 ticks)
{
    return (ticks / 91UL) * 5000UL + ((ticks % 91UL) * 5000UL) / 91UL;
}

static int contains_text(const char *data, u32 len, const char *needle)
{
    u32 needle_len;
    u32 index;
    u32 offset;

    needle_len = (u32)strlen(needle);
    if (needle_len == 0 || len < needle_len) {
        return 0;
    }

    for (index = 0; index + needle_len <= len; index++) {
        for (offset = 0; offset < needle_len; offset++) {
            if (data[index + offset] != needle[offset]) {
                break;
            }
        }
        if (offset == needle_len) {
            return 1;
        }
    }

    return 0;
}

int command_not_found_matches(const char *output, u32 len)
{
    return contains_text(output, len, "Bad command or file name") ||
           contains_text(output, len, "Bad command or filename");
}

static int read_exit_code(void)
{
    union REGS inregs;
    union REGS outregs;

    inregs.h.ah = 0x4D;
    intdos(&inregs, &outregs);
    return (int)outregs.h.al;
}

static int dup_handle(int handle)
{
    union REGS inregs;
    union REGS outregs;

    inregs.h.ah = 0x45;
    inregs.x.bx = (unsigned)handle;
    intdos(&inregs, &outregs);
    if (outregs.x.cflag) {
        return -1;
    }
    return (int)outregs.x.ax;
}

static int force_dup(int source, int target)
{
    union REGS inregs;
    union REGS outregs;

    inregs.h.ah = 0x46;
    inregs.x.bx = (unsigned)source;
    inregs.x.cx = (unsigned)target;
    intdos(&inregs, &outregs);
    return outregs.x.cflag ? 0 : 1;
}

static void close_handle(int handle)
{
    union REGS inregs;
    union REGS outregs;

    inregs.h.ah = 0x3E;
    inregs.x.bx = (unsigned)handle;
    intdos(&inregs, &outregs);
}

static int run_batch(void)
{
    const char *comspec;
    FILE *error_file;
    int error_handle;
    int saved_error;
    int spawn_result;

    /* The batch captures stdout. FreeDOS writes shell errors to handle 2. */
    comspec = comspec_path();
    error_file = fopen(ERROR_PATH, "wb");
    error_handle = error_file != NULL ? fileno(error_file) : -1;
    saved_error = dup_handle(2);
    if (error_handle >= 0) {
        force_dup(error_handle, 2);
    }

    spawn_result = spawnl(P_WAIT, comspec, comspec, "/C", BATCH_PATH,
                          (char *)0);

    if (saved_error >= 0) {
        force_dup(saved_error, 2);
        close_handle(saved_error);
    }
    if (error_file != NULL) {
        fclose(error_file);
    }

    return spawn_result;
}

static void append_error_output(void)
{
    FILE *output;
    FILE *error;
    int value;

    /* Merge the captured diagnostics after the command output. */
    error = fopen(ERROR_PATH, "rb");
    if (error == NULL) {
        return;
    }
    output = fopen(OUTPUT_PATH, "ab");
    if (output == NULL) {
        fclose(error);
        return;
    }

    while ((value = fgetc(error)) != EOF) {
        fputc(value, output);
    }

    fclose(error);
    fclose(output);
}

static void remove_temp_files(void)
{
    remove(BATCH_PATH);
    remove(OUTPUT_PATH);
    remove(ERROR_PATH);
}

static int stream_output(struct session *s, u32 limit,
                         struct exec_result *result)
{
    FILE *file;
    u32 output_bytes;
    u16 overlap;
    int send_ok;

    if (limit > MAX_OUTPUT_BYTES) {
        limit = MAX_OUTPUT_BYTES;
    }

    output_bytes = 0;
    overlap = 0;
    send_ok = 1;
    file = fopen(OUTPUT_PATH, "rb");
    if (file == NULL) {
        result->output_bytes = 0;
        return 1;
    }

    for (;;) {
        u32 remaining;
        u16 chunk;
        u16 count;

        remaining = limit - output_bytes;
        if (remaining == 0) {
            if (fgetc(file) != EOF) {
                result->truncated = 1;
            }
            break;
        }

        chunk = (u16)(remaining > (u32)FRAME_CHUNK ?
                      (u32)FRAME_CHUNK : remaining);
        count = (u16)fread(output_buffer, 1, chunk, file);
        if (count == 0) {
            break;
        }

        if (!result->command_not_found) {
            u32 combined;
            u16 tail;

            combined = (u32)overlap + (u32)count;
            memcpy(scan_buffer, match_tail, overlap);
            memcpy(scan_buffer + overlap, output_buffer, count);
            if (command_not_found_matches(scan_buffer, combined)) {
                result->command_not_found = 1;
            }

            tail = combined > (u32)MATCH_OVERLAP ?
                   (u16)MATCH_OVERLAP : (u16)combined;
            memcpy(match_tail, scan_buffer + combined - tail, tail);
            overlap = tail;
        }

        output_bytes += count;
        if (session_send(s, FRAME_EXEC_DATA, output_buffer, count) != 0) {
            send_ok = 0;
            break;
        }
    }

    fclose(file);
    result->output_bytes = output_bytes;
    return send_ok;
}

int exec_run(struct session *s, const struct exec_request *request,
             struct exec_result *result)
{
    long start_ticks;
    long end_ticks;
    int spawn_result;
    char end_json[128];
    u16 end_length;
    int ok;

    memset(result, 0, sizeof(*result));

    if (!command_is_valid(request->command)) {
        if (strlen(request->command) > MAX_COMMAND_CHARS) {
            send_error(s, "TOO_LONG",
                       "command or path exceeds the DOS limit",
                       s->request_id);
        } else {
            send_error(s, "INTERNAL", "agent internal error", s->request_id);
        }
        return -1;
    }

    if (!write_batch(request)) {
        send_error(s, "INTERNAL", "agent internal error", s->request_id);
        remove_temp_files();
        return -1;
    }

    start_ticks = bios_ticks();
    spawn_result = run_batch();
    end_ticks = bios_ticks();

    if (spawn_result < 0) {
        result->exit_code = 1;
    } else {
        /* spawnl may already have consumed the INT 21h/AH=4Dh result. */
        result->exit_code = read_exit_code();
        if (result->exit_code == 0) {
            result->exit_code = spawn_result;
        }
    }

    if (end_ticks >= start_ticks) {
        result->duration_ms = ticks_to_ms((u32)(end_ticks - start_ticks));
    } else {
        result->duration_ms = 0;
    }

    append_error_output();

    result->output_bytes = 0;
    ok = stream_output(s, request->max_output_bytes, result);
    if (ok) {
        if (!json_write_exec_end(end_json, (u16)sizeof(end_json), result)) {
            ok = 0;
        } else {
            end_length = (u16)strlen(end_json);
            if (session_send(s, FRAME_EXEC_END, (const u8 *)end_json,
                             end_length) != 0) {
                ok = 0;
            }
        }
    }

    remove_temp_files();

    return ok ? 0 : -1;
}

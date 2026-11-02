#include "dosker.h"

#include <stdio.h>
#include <string.h>

#define ATTRIBUTE_DIRECTORY 0x10
#define CHUNK_BYTES MAX_PAYLOAD

static FILE *put_file;
static char put_temp[128];
static u8 get_buffer[CHUNK_BYTES];

int put_begin(const char *path, u32 size, int overwrite, char *temp_path,
              u16 temp_len)
{
    u8 attributes;

    if (put_file != NULL || put_temp[0] != '\0') {
        put_abort(put_temp);
    }

    if (!path_is_valid(path)) {
        return TRANSFER_BAD_PATH;
    }

    if (file_exists(path)) {
        if (!overwrite) {
            return TRANSFER_BAD_PATH;
        }
        if (file_attributes(path, &attributes) &&
            (attributes & ATTRIBUTE_DIRECTORY) != 0) {
            return TRANSFER_BAD_PATH;
        }
    }

    if (size > MAX_TRANSFER_BYTES) {
        return TRANSFER_TOO_LARGE;
    }

    if (free_space(path) < size) {
        return TRANSFER_NO_SPACE;
    }

    if (!temp_path_for(path, temp_path, temp_len)) {
        return TRANSFER_INTERNAL;
    }

    put_file = fopen(temp_path, "wb");
    if (put_file == NULL) {
        return TRANSFER_INTERNAL;
    }
    strcpy(put_temp, temp_path);

    return 0;
}

int put_chunk(const u8 *data, u16 len)
{
    if (put_file == NULL) {
        return 0;
    }

    if ((u32)fwrite(data, 1, len, put_file) != (u32)len) {
        return 0;
    }

    return 1;
}

int put_finish(const char *temp_path, const char *target)
{
    int result;

    if (put_file != NULL) {
        if (fclose(put_file) != 0) {
            put_file = NULL;
            return -1;
        }
        put_file = NULL;
    }

    result = file_rename(temp_path, target);
    if (result != 0 && file_exists(target)) {
        /* DOS rename cannot replace an existing file. Delete the target
           only after the first rename fails. The target is missing
           between the delete and the retry. */
        if (file_delete(target) == 0) {
            result = file_rename(temp_path, target);
        }
    }

    if (result == 0) {
        put_temp[0] = '\0';
    }

    return result;
}

void put_abort(const char *temp_path)
{
    if (put_file != NULL) {
        fclose(put_file);
        put_file = NULL;
    }

    file_delete(temp_path);
    put_temp[0] = '\0';
}

int get_begin(const char *path, u32 *size)
{
    u8 attributes;

    if (!path_is_valid(path)) {
        return TRANSFER_BAD_PATH;
    }

    if (!file_exists(path)) {
        return TRANSFER_NO_FILE;
    }

    if (file_attributes(path, &attributes) &&
        (attributes & ATTRIBUTE_DIRECTORY) != 0) {
        return TRANSFER_NO_FILE;
    }

    if (!file_size(path, size)) {
        return TRANSFER_NO_FILE;
    }

    if (*size > MAX_TRANSFER_BYTES) {
        return TRANSFER_TOO_LARGE;
    }

    return 0;
}

static int send_get_end(struct session *s, int ok, u32 size)
{
    char json[128];
    u16 length;

    if (!json_write_get_end(json, (u16)sizeof(json), ok, size)) {
        return -1;
    }

    length = (u16)strlen(json);
    return session_send(s, FRAME_GET_END, (const u8 *)json, length);
}

int get_stream(struct session *s, const char *path, u32 size)
{
    FILE *file;
    u32 sent;
    int ok;

    file = fopen(path, "rb");
    if (file == NULL) {
        send_get_end(s, 0, 0);
        return -1;
    }

    sent = 0;
    ok = 1;
    while (sent < size) {
        u32 remaining;
        u16 chunk;
        u16 count;

        remaining = size - sent;
        chunk = (u16)(remaining > (u32)CHUNK_BYTES ?
                      (u32)CHUNK_BYTES : remaining);

        count = (u16)fread(get_buffer, 1, chunk, file);
        if (count == 0) {
            ok = 0;
            break;
        }

        if (session_send(s, FRAME_GET_DATA, get_buffer, count) != 0) {
            fclose(file);
            return -1;
        }

        sent += count;
    }

    fclose(file);

    if (!ok) {
        send_get_end(s, 0, 0);
        return -1;
    }

    return send_get_end(s, 1, sent);
}

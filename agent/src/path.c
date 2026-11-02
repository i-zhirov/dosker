#include "dosker.h"

#include <string.h>

static int is_name_char(char value)
{
    if (value >= 'A' && value <= 'Z') {
        return 1;
    }
    if (value >= '0' && value <= '9') {
        return 1;
    }

    switch (value) {
    case '$':
    case '#':
    case '@':
    case '!':
    case '%':
    case '&':
    case '\'':
    case '(':
    case ')':
    case '-':
    case '{':
    case '}':
    case '~':
    case '^':
    case '_':
    case '`':
        return 1;
    default:
        return 0;
    }
}

int path_is_valid(const char *path)
{
    const char *cursor;
    u16 length;

    length = (u16)strlen(path);
    if (length == 0 || length > MAX_COMMAND_CHARS) {
        return 0;
    }

    cursor = path;
    if (cursor[0] >= 'A' && cursor[0] <= 'Z' && cursor[1] == ':') {
        cursor += 2;
    }
    if (*cursor != '\\') {
        return 0;
    }
    cursor++;

    if (*cursor == '\0') {
        return 1;
    }

    for (;;) {
        u16 name_length;
        u16 extension_length;

        name_length = 0;
        while (is_name_char(*cursor) && name_length < 9) {
            cursor++;
            name_length++;
        }
        if (name_length == 0 || name_length > 8) {
            return 0;
        }

        if (*cursor == '.') {
            cursor++;
            extension_length = 0;
            while (is_name_char(*cursor) && extension_length < 4) {
                cursor++;
                extension_length++;
            }
            if (extension_length == 0 || extension_length > 3) {
                return 0;
            }
        }

        if (*cursor == '\\') {
            cursor++;
            if (*cursor == '\0') {
                return 0;
            }
            continue;
        }
        if (*cursor == '\0') {
            return 1;
        }
        return 0;
    }
}

int command_is_valid(const char *command)
{
    u16 length;

    length = (u16)strlen(command);
    return length >= 1 && length <= MAX_COMMAND_CHARS;
}

int path_parent(const char *path, char *directory, u16 directory_len)
{
    const char *last;
    u16 length;

    last = strrchr(path, '\\');
    if (last == NULL) {
        return 0;
    }

    if (last == path) {
        if (directory_len < 2) {
            return 0;
        }
        directory[0] = '\\';
        directory[1] = '\0';
        return 1;
    }

    if (last == path + 2 && path[1] == ':') {
        if (directory_len < 4) {
            return 0;
        }
        directory[0] = path[0];
        directory[1] = ':';
        directory[2] = '\\';
        directory[3] = '\0';
        return 1;
    }

    length = (u16)(last - path);
    if ((u32)length + 1 > (u32)directory_len) {
        return 0;
    }
    memcpy(directory, path, length);
    directory[length] = '\0';
    return 1;
}

int path_leaf(const char *path, char *leaf, u16 leaf_len)
{
    const char *last;
    const char *name;
    u16 length;

    last = strrchr(path, '\\');
    if (last == NULL) {
        return 0;
    }

    name = last + 1;
    length = (u16)strlen(name);
    if (length == 0 || (u32)length + 1 > (u32)leaf_len) {
        return 0;
    }
    memcpy(leaf, name, length);
    leaf[length] = '\0';
    return 1;
}

int path_join(char *output, u16 output_len, const char *directory,
              const char *name)
{
    u16 directory_length;
    u16 name_length;
    u16 needs_separator;
    u32 total;

    directory_length = (u16)strlen(directory);
    name_length = (u16)strlen(name);
    needs_separator = 0;
    if (directory_length > 0 && directory[directory_length - 1] != '\\') {
        needs_separator = 1;
    }

    total = (u32)directory_length + (u32)needs_separator +
            (u32)name_length + 1;
    if (total > (u32)output_len) {
        return 0;
    }

    memcpy(output, directory, directory_length);
    if (needs_separator) {
        output[directory_length] = '\\';
    }
    memcpy(output + directory_length + needs_separator, name, name_length);
    output[total - 1] = '\0';
    return 1;
}

#include "dosker.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CONFIG_PATH "C:\\DOSKER\\CONFIG.INI"
#define CONFIG_LINE_CHARS 128
#define DEFAULT_CONTROL_PORT 2
#define DEFAULT_BAUD 115200UL

void config_defaults(struct config *cfg)
{
    cfg->control_port = (u8)DEFAULT_CONTROL_PORT;
    cfg->baud = (u32)DEFAULT_BAUD;
}

static void trim(char *text)
{
    char *start;
    u16 length;

    start = text;
    while (*start == ' ' || *start == '\t') {
        start++;
    }
    if (start != text) {
        memmove(text, start, strlen(start) + 1);
    }

    length = (u16)strlen(text);
    while (length > 0) {
        if (text[length - 1] != '\r' && text[length - 1] != '\n' &&
            text[length - 1] != ' ' && text[length - 1] != '\t') {
            break;
        }
        text[--length] = '\0';
    }
}

static void uppercase(char *text)
{
    while (*text != '\0') {
        if (*text >= 'a' && *text <= 'z') {
            *text = (char)(*text - 'a' + 'A');
        }
        text++;
    }
}

static int valid_baud(u32 baud)
{
    u16 divisor;

    return baud_divisor(baud, &divisor) == 0;
}

int config_load(struct config *cfg)
{
    FILE *file;
    char line[CONFIG_LINE_CHARS];
    char key[CONFIG_LINE_CHARS];
    char value[CONFIG_LINE_CHARS];
    char *separator;
    long port;
    long baud;

    config_defaults(cfg);

    file = fopen(CONFIG_PATH, "r");
    if (file == NULL) {
        return 1;
    }

    while (fgets(line, CONFIG_LINE_CHARS, file) != NULL) {
        trim(line);
        if (line[0] == '\0' || line[0] == ';' || line[0] == '#') {
            continue;
        }

        separator = strchr(line, '=');
        if (separator == NULL) {
            continue;
        }

        *separator = '\0';
        strcpy(key, line);
        strcpy(value, separator + 1);
        trim(key);
        trim(value);
        uppercase(key);

        if (strcmp(key, "CONTROL_PORT") == 0) {
            port = atol(value);
            if (port >= 1L && port <= 4L) {
                cfg->control_port = (u8)port;
            }
        } else if (strcmp(key, "BAUD") == 0) {
            baud = atol(value);
            if (baud > 0L && valid_baud((u32)baud)) {
                cfg->baud = (u32)baud;
            }
        }
    }

    fclose(file);
    return 1;
}

u16 serial_base(u8 port)
{
    switch (port) {
    case 1:
        return 0x3F8;
    case 2:
        return 0x2F8;
    case 3:
        return 0x3E8;
    case 4:
        return 0x2E8;
    default:
        return 0;
    }
}

int baud_divisor(u32 baud, u16 *divisor)
{
    switch (baud) {
    case 9600UL:
        *divisor = 12;
        return 0;
    case 19200UL:
        *divisor = 6;
        return 0;
    case 38400UL:
        *divisor = 3;
        return 0;
    case 57600UL:
        *divisor = 2;
        return 0;
    case 115200UL:
        *divisor = 1;
        return 0;
    default:
        return -1;
    }
}

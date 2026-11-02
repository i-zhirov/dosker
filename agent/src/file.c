#include "dosker.h"

#include <dos.h>
#include <stdio.h>
#include <string.h>

#define FIND_ALL (unsigned)(_A_RDONLY | _A_HIDDEN | _A_SYSTEM | \
                            _A_SUBDIR | _A_ARCH)
#define TEMP_NAME "DOSKER.TMP"
#define TEMP_NAME_FALLBACK "DOSKER.TM1"
#define TIMESTAMP_CHARS 20

static int leaf_is_temp(const char *leaf)
{
    const char *expected;
    u16 index;

    expected = TEMP_NAME;
    for (index = 0; expected[index] != '\0'; index++) {
        char value;
        char wanted;

        value = leaf[index];
        wanted = expected[index];
        if (value >= 'a' && value <= 'z') {
            value = (char)(value - 'a' + 'A');
        }
        if (value != wanted) {
            return 0;
        }
    }

    return leaf[index] == '\0';
}

int file_exists(const char *path)
{
    unsigned attributes;

    return _dos_getfileattr(path, &attributes) == 0;
}

int file_size(const char *path, u32 *size)
{
    struct find_t info;
    u32 value;

    if (_dos_findfirst(path, FIND_ALL, &info) != 0) {
        return 0;
    }

    value = (u32)info.size;
    _dos_findclose(&info);

    *size = value;
    return 1;
}

int file_attributes(const char *path, u8 *attributes)
{
    unsigned value;

    if (_dos_getfileattr(path, &value) != 0) {
        return 0;
    }

    *attributes = (u8)value;
    return 1;
}

int file_timestamp(const char *path, char *output, u16 output_len)
{
    struct find_t info;
    u16 date;
    u16 time;
    unsigned year;
    unsigned month;
    unsigned day;
    unsigned hour;
    unsigned minute;
    unsigned second;

    if (_dos_findfirst(path, FIND_ALL, &info) != 0) {
        return 0;
    }

    date = (u16)info.wr_date;
    time = (u16)info.wr_time;
    _dos_findclose(&info);

    if (output_len < TIMESTAMP_CHARS + 1) {
        return 0;
    }

    year = 1980U + ((unsigned)date >> 9);
    month = ((unsigned)date >> 5) & 0x0FU;
    day = (unsigned)date & 0x1FU;
    hour = ((unsigned)time >> 11) & 0x1FU;
    minute = ((unsigned)time >> 5) & 0x3FU;
    second = ((unsigned)time & 0x1FU) * 2U;

    sprintf(output, "%04u-%02u-%02uT%02u:%02u:%02uZ", year, month, day,
            hour, minute, second);
    return 1;
}

int file_delete(const char *path)
{
    return remove(path) == 0 ? 0 : -1;
}

int file_rename(const char *from, const char *to)
{
    union REGS inregs;
    union REGS outregs;
    struct SREGS segregs;
    char *source;
    char *target;

    source = (char *)from;
    target = (char *)to;

    memset(&inregs, 0, sizeof(inregs));
    memset(&outregs, 0, sizeof(outregs));
    segread(&segregs);

    inregs.h.ah = 0x56;
    inregs.x.dx = FP_OFF(source);
    inregs.x.di = FP_OFF(target);
    segregs.es = segregs.ds;

    intdosx(&inregs, &outregs, &segregs);

    return outregs.x.cflag ? -1 : 0;
}

u32 free_space(const char *path)
{
    union REGS inregs;
    union REGS outregs;
    unsigned drive;
    u32 sectors_per_cluster;
    u32 free_clusters;
    u32 bytes_per_sector;

    memset(&inregs, 0, sizeof(inregs));

    drive = 0;
    if (path[1] == ':') {
        char letter;

        letter = path[0];
        if (letter >= 'a' && letter <= 'z') {
            letter = (char)(letter - 'a' + 'A');
        }
        if (letter >= 'A' && letter <= 'Z') {
            drive = (unsigned)(letter - 'A' + 1);
        }
    }

    inregs.h.ah = 0x36;
    inregs.h.dl = (u8)drive;
    intdos(&inregs, &outregs);

    if (outregs.x.ax == 0xFFFFU) {
        return 0;
    }

    sectors_per_cluster = (u32)outregs.x.ax;
    free_clusters = (u32)outregs.x.bx;
    bytes_per_sector = (u32)outregs.x.cx;

    return sectors_per_cluster * free_clusters * bytes_per_sector;
}

int temp_path_for(const char *target, char *output, u16 output_len)
{
    char directory[128];
    char leaf[16];
    const char *name;

    if (!path_parent(target, directory, (u16)sizeof(directory))) {
        return 0;
    }
    if (!path_leaf(target, leaf, (u16)sizeof(leaf))) {
        return 0;
    }

    name = leaf_is_temp(leaf) ? TEMP_NAME_FALLBACK : TEMP_NAME;
    return path_join(output, output_len, directory, name);
}

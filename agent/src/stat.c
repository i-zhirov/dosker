#include "dosker.h"

#include <dos.h>
#include <string.h>

#define FIND_ALL (unsigned)(_A_RDONLY | _A_HIDDEN | _A_SYSTEM | \
                            _A_SUBDIR | _A_ARCH)

int stat_path(const char *path, struct stat_result *result)
{
    struct find_t info;
    u8 attributes;
    u32 size;

    memset(result, 0, sizeof(*result));

    if (_dos_findfirst(path, FIND_ALL, &info) != 0) {
        return 0;
    }

    attributes = (u8)info.attrib;
    size = (u32)info.size;
    _dos_findclose(&info);

    result->exists = 1;
    result->attributes = attributes;

    if ((attributes & _A_SUBDIR) != 0) {
        result->directory = 1;
        result->size = 0;
    } else {
        result->directory = 0;
        result->size = size;
    }

    file_timestamp(path, result->timestamp, (u16)sizeof(result->timestamp));
    return 1;
}

#include "dosker.h"

u32 crc32_update(u32 crc, const u8 *data, u16 len)
{
    u16 i;
    u8 bit;

    for (i = 0; i < len; i++) {
        crc ^= (u32)data[i];
        for (bit = 0; bit < 8; bit++) {
            if (crc & 1UL) {
                crc = (crc >> 1) ^ 0xEDB88320UL;
            } else {
                crc >>= 1;
            }
        }
    }
    return crc;
}

u32 crc32(const u8 *data, u16 len)
{
    return crc32_update(0xFFFFFFFFUL, data, len) ^ 0xFFFFFFFFUL;
}

#include <i86.h>

#define SECTOR_BYTES 512
#define BIOS_DRIVE 0x80
#define BOOT_CYLINDER 4
#define BOOT_HEAD 0
#define BOOT_SECTOR 1
#define VOLUME_LABEL_OFFSET 43

static unsigned char sector[SECTOR_BYTES];

static void prepare_request(union REGS *regs, struct SREGS *segs, unsigned char command)
{
    regs->h.ah = command;
    regs->h.al = 0x01;
    regs->h.ch = (unsigned char)BOOT_CYLINDER;
    regs->h.cl = (unsigned char)(((BOOT_CYLINDER >> 8) << 6) | BOOT_SECTOR);
    regs->h.dh = BOOT_HEAD;
    regs->h.dl = BIOS_DRIVE;
    regs->x.bx = (unsigned short)FP_OFF(sector);
    segs->es = (unsigned short)FP_SEG(sector);
}

int main(void)
{
    union REGS regs;
    struct SREGS segs;

    segread(&segs);

    prepare_request(&regs, &segs, 0x02);
    int86x(0x13, &regs, &regs, &segs);
    if (regs.x.cflag) {
        return 1;
    }

    sector[VOLUME_LABEL_OFFSET] ^= 0x01;

    prepare_request(&regs, &segs, 0x03);
    int86x(0x13, &regs, &regs, &segs);
    if (regs.x.cflag) {
        return 1;
    }

    return 0;
}

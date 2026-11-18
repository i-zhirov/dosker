#include <i86.h>

int main(void)
{
    union REGS regs;

    for (;;) {
        regs.x.ax = 0x0000;
        int86(0x19, &regs, &regs);
    }
}

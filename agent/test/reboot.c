/*
 * Reset the guest by loading an empty IDT and raising an exception. The BIOS
 * bootstrap interrupt is not reliable under every emulated BIOS, and this path
 * is seen by QEMU as a system reset, which the build VM maps to GuestRebooted.
 */
static void reboot_cpu(void);

#pragma aux reboot_cpu = \
    "cli" \
    "xor ax, ax" \
    "mov ds, ax" \
    "mov es, ax" \
    "mov ss, ax" \
    "mov sp, 0x7c00" \
    "push 0" \
    "push 0" \
    "mov bx, sp" \
    "lidt [bx]" \
    "int 3" \
    "hlt" \
    modify [ax bx];

int main(void)
{
    reboot_cpu();
    return 0;
}

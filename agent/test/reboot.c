/*
 * Reset the guest through the BIOS reset vector. The BIOS bootstrap interrupt
 * is not reliable under every emulated BIOS, but the far jump is seen by QEMU
 * as a system reset, which the build VM maps to GuestRebooted. The far jump is
 * 8086-safe; `lidt` and immediate pushes are not available in this target.
 */
static void reboot_cpu(void);

#pragma aux reboot_cpu = \
    "mov ax, 0ffffh" \
    "push ax" \
    "xor ax, ax" \
    "push ax" \
    "retf" \
    modify [ax];

int main(void)
{
    reboot_cpu();
    return 0;
}

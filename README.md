# SigmaOS-xtask

SigmaOS build runner and QEMU orchestrator for the custom-kernel
components (`sigma-kernel`, `sigma-sh`, `initramfs`, `sigma-bootloader`).

Commands:

- `xtask build-kernel` — build sigma-kernel for `x86_64-unknown-none`
- `xtask build-userspace` — build userspace binaries (sigma-sh, …)
- `xtask initramfs` — pack user binaries into a cpio archive
- `xtask kernel-elf` — produce `kernel.elf` with bootloader header
- `xtask qemu` / `xtask qemu-debug` — boot in QEMU (serial+VGA / GDB stub)
- `xtask test-qemu` — run the kernel test suite under QEMU
- `xtask status` — report toolchain/binary presence

Note: this orchestrates the *experimental* custom-kernel stack. The
shipped SigmaOS product is the Arch-based ISO in
[SigmaOS-ISO](../SigmaOS-ISO/README.md), which uses the stock kernel
and does not depend on xtask.

## See Also

- [SigmaOS-sigma-kernel](../SigmaOS-sigma-kernel/README.md) — kernel it builds
- [SigmaOS-sigma-sh](../SigmaOS-sigma-sh/README.md) — userspace shell it packs
- [SigmaOS-initramfs](../SigmaOS-initramfs/README.md) — initramfs builder
- [SigmaOS-ISO](../SigmaOS-ISO/README.md) — the shipped Arch-based product

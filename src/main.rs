//! xtask — SigmaOS build runner and QEMU orchestrator.
//!
//! Commands:
//!   xtask build-kernel     build sigma-kernel for x86_64-unknown-none
//!   xtask build-userspace  build sigma-sh (and future user binaries)
//!   xtask initramfs        pack user binaries into a cpio archive
//!   xtask kernel-elf       produce kernel.elf + bootloader header
//!   xtask qemu             boot the kernel in QEMU (serial + vga)
//!   xtask qemu-debug       same, with GDB stub on :1234
//!   xtask test-qemu        run the kernel test suite under QEMU
//!   xtask status           report toolchain/binary presence

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::Command as Proc;

#[derive(Parser)]
#[command(name = "xtask", about = "SigmaOS build & QEMU orchestrator")]
struct Cli {
    /// Repo root (defaults to parent of this crate, or $SIGMAOS_SRC)
    #[arg(long, global = true)]
    root: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build the kernel for x86_64-unknown-none.
    BuildKernel,
    /// Build userspace binaries (sigma-sh).
    BuildUserspace,
    /// Pack initramfs cpio from user binaries.
    Initramfs {
        /// Output archive path.
        #[arg(long, default_value = "target/initramfs.cpio")]
        out: PathBuf,
    },
    /// Produce kernel ELF with Multiboot2 header embedded.
    KernelElf {
        /// Output path.
        #[arg(long, default_value = "target/kernel.elf")]
        out: PathBuf,
    },
    /// Boot kernel in QEMU (serial console + VGA).
    Qemu {
        /// Kernel ELF path.
        #[arg(long, default_value = "target/kernel.elf")]
        kernel: PathBuf,
        /// Initramfs path.
        #[arg(long, default_value = "target/initramfs.cpio")]
        initramfs: PathBuf,
        /// RAM in MiB.
        #[arg(long, default_value_t = 512)]
        mem: u32,
        /// Timeout seconds (0 = run forever).
        #[arg(long, default_value_t = 0)]
        timeout: u32,
    },
    /// Boot with GDB stub on localhost:1234.
    QemuDebug {
        #[arg(long, default_value = "target/kernel.elf")]
        kernel: PathBuf,
    },
    /// Run kernel tests under QEMU via isa-debug-exit.
    TestQemu,
    /// Report which toolchain pieces are present.
    Status,
}

fn root(cli_root: &Option<PathBuf>) -> PathBuf {
    if let Some(r) = cli_root {
        return r.clone();
    }
    if let Ok(src) = std::env::var("SIGMAOS_SRC") {
        return PathBuf::from(src);
    }
    // xtask lives in <root>/SigmaOS-xtask or is invoked from root
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join("SigmaOS-sigma-kernel").is_dir() {
        cwd
    } else {
        cwd.join("..")
            .canonicalize()
            .unwrap_or_else(|_| cwd.clone())
    }
}

fn repo(root: &Path, name: &str) -> PathBuf {
    root.join(format!("SigmaOS-{}", name))
}

fn run(cmd: &mut Proc, what: &str) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn {}: {}", what, e))?;
    if !status.success() {
        return Err(format!("{} exited with {}", what, status));
    }
    Ok(())
}

fn cargo_build(dir: &Path, target: Option<&str>) -> Result<(), String> {
    let mut c = Proc::new("cargo");
    c.arg("build").arg("--release").current_dir(dir);
    if let Some(t) = target {
        c.arg("--target").arg(t);
    }
    run(&mut c, "cargo build")
}

fn cmd_build_kernel(root: &Path) -> Result<(), String> {
    println!("== building sigma-kernel (x86_64-unknown-none) ==");
    cargo_build(&repo(root, "sigma-kernel"), Some("x86_64-unknown-none"))
}

fn cmd_build_userspace(root: &Path) -> Result<(), String> {
    println!("== building sigma-sh ==");
    let sh = repo(root, "sigma-sh");
    let mut c0 = Proc::new("cargo");
    c0.arg("build").arg("--release").arg("--bin").arg("sigma-sh")
        .current_dir(&sh);
    run(&mut c0, "cargo build sigma-sh")?;
    println!("== building sigma-init (x86_64-unknown-none) ==");
    let sh = repo(root, "sigma-sh");
    let mut c = Proc::new("cargo");
    c.arg("build").arg("--release")
        .arg("--bin").arg("sigma-init")
        .arg("--target").arg("x86_64-unknown-none")
        .current_dir(&sh);
    run(&mut c, "cargo build sigma-init")
}

fn cmd_initramfs(root: &Path, out: &Path) -> Result<(), String> {
    println!("== packing initramfs -> {} ==", out.display());
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    // stage known userspace binaries
    for (name, rel) in [
        ("bin/sigma-sh", "sigma-sh/target/release/sigma-sh"),
        ("init", "sigma-sh/target/x86_64-unknown-none/release/sigma-init"),
    ] {
        let p = repo(root, rel.split('/').next().unwrap()).join(
            rel.split_once('/').map(|(_, r)| r).unwrap_or(rel),
        );
        if p.exists() {
            entries.push((name.to_string(), p));
        }
    }
    if entries.is_empty() {
        return Err("no userspace binaries found — run `xtask build-userspace` first".into());
    }
    // cpio newc writer
    let mut buf = Vec::new();
    for (arcname, src) in &entries {
        let data = std::fs::read(src).map_err(|e| format!("read {}: {}", src.display(), e))?;
        push_newc(&mut buf, arcname, &data, 0o755);
    }
    push_newc_trailer(&mut buf);
    std::fs::write(out, &buf).map_err(|e| e.to_string())?;
    println!("   {} files, {} bytes", entries.len(), buf.len());
    Ok(())
}

fn hex8_usize(v: usize) -> [u8; 8] {
    // cpio newc fields are 8 ASCII hex digits, not binary.
    let s = format!("{:08x}", v);
    let mut b = [0u8; 8];
    b.copy_from_slice(s.as_bytes());
    b
}

fn push_newc(buf: &mut Vec<u8>, name: &str, data: &[u8], mode: u32) {
    push_newc_ino(buf, name, data, mode, 1);
}

fn push_newc_ino(buf: &mut Vec<u8>, name: &str, data: &[u8], mode: u32, ino: usize) {
    // newc header: magic(6) ino(8) mode(8) uid(8) gid(8) nlink(8)
    // mtime(8) filesize(8) devmaj(8) devmin(8) rdevmaj(8) rdevmin(8)
    // namesize(8) check(8) = 110 bytes, then name NUL, pad to 4
    let namesize = name.len() + 1;
    let mut hdr = Vec::with_capacity(110);
    hdr.extend_from_slice(b"070701");
    let fields: [usize; 13] = [
        ino,
        mode as usize,
        0, 0, 1, // uid gid nlink
        0,                 // mtime
        data.len(),        // filesize
        0, 0, 0, 0,        // dev/rdev
        namesize,          // namesize
        0,                 // check
    ];
    for f in fields {
        hdr.extend_from_slice(&hex8_usize(f));
    }
    assert_eq!(hdr.len(), 110);
    buf.extend_from_slice(&hdr);
    buf.extend_from_slice(name.as_bytes());
    buf.push(0);
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
    buf.extend_from_slice(data);
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

fn push_newc_trailer(buf: &mut Vec<u8>) {
    push_newc_ino(buf, "TRAILER!!!", &[], 0, 0);
    // pad archive to 512
    while buf.len() % 512 != 0 {
        buf.push(0);
    }
}

fn cmd_kernel_elf(root: &Path, out: &Path) -> Result<(), String> {
    println!("== building sigma-kernel ELF (x86_64-unknown-none) ==");
    let kdir = repo(root, "sigma-kernel");
    let mut c = Proc::new("cargo");
    c.arg("build")
        .arg("--release")
        .arg("--target")
        .arg("x86_64-unknown-none")
        .arg("--bin")
        .arg("sigma-kernel")
        .current_dir(&kdir);
    run(&mut c, "cargo build kernel")?;

    let elf = kdir.join("target/x86_64-unknown-none/release/sigma-kernel");
    if !elf.exists() {
        return Err(format!("kernel ELF not found at {}", elf.display()));
    }
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    std::fs::copy(&elf, out).map_err(|e| format!("copy ELF: {}", e))?;
    println!("   kernel ELF -> {}", out.display());
    Ok(())
}

fn qemu_args(kernel: &Path, initramfs: &Path, mem: u32) -> Vec<String> {
    vec![
        "-machine".into(),
        "q35".into(),
        "-m".into(),
        format!("{}M", mem),
        "-kernel".into(),
        kernel.display().to_string(),
        "-initrd".into(),
        initramfs.display().to_string(),
        "-serial".into(),
        "stdio".into(),
        "-display".into(),
        "none".into(),
        "-no-reboot".into(),
    ]
}

fn check_qemu() -> Result<(), String> {
    let ok = Proc::new("qemu-system-x86_64")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        return Err("qemu-system-x86_64 not found on PATH".into());
    }
    Ok(())
}

fn cmd_qemu(kernel: &Path, initramfs: &Path, mem: u32, timeout: u32) -> Result<(), String> {
    check_qemu()?;
    if !kernel.exists() {
        return Err(format!("kernel not found: {} — run `xtask kernel-elf`", kernel.display()));
    }
    if !initramfs.exists() {
        return Err(format!(
            "initramfs not found: {} — run `xtask initramfs`",
            initramfs.display()
        ));
    }
    let mut args = qemu_args(kernel, initramfs, mem);
    if timeout > 0 {
        args.push("-device".into());
        args.push("isa-debug-exit,iobase=0xf4,iosize=0x04".into());
    }
    println!("qemu-system-x86_64 {}", args.join(" "));
    let mut c = Proc::new("qemu-system-x86_64");
    c.args(&args);
    if timeout > 0 {
        // wrap in `timeout` so the VM exits cleanly
        let mut t = Proc::new("timeout");
        t.arg(format!("{}s", timeout)).arg("qemu-system-x86_64").args(&args);
        run(&mut t, "qemu")
    } else {
        let st = c
            .status()
            .map_err(|e| format!("qemu spawn: {}", e))?;
        if !st.success() {
            return Err(format!("qemu exited {}", st));
        }
        Ok(())
    }
}

fn cmd_status(root: &Path) {
    let checks: Vec<(&str, Proc)> = vec![
        ("cargo", {
            let mut c = Proc::new("cargo");
            c.arg("--version");
            c
        }),
        ("rustc x86_64-unknown-none target", {
            let mut c = Proc::new("rustc");
            c.arg("--print").arg("target-list");
            c
        }),
        ("qemu-system-x86_64", {
            let mut c = Proc::new("qemu-system-x86_64");
            c.arg("--version");
            c
        }),
        ("gdb", {
            let mut c = Proc::new("gdb");
            c.arg("--version");
            c
        }),
        ("ovmf dirs", {
            let mut c = Proc::new("sh");
            c.arg("-c").arg("ls /usr/share/OVMF /usr/share/edk2 /usr/share/qemu 2>/dev/null | head -1");
            c
        }),
    ];
    for (name, mut c) in checks {
        let ok = c.output().map(|o| o.status.success()).unwrap_or(false);
        println!("{:<38} {}", name, if ok { "ok" } else { "MISSING" });
    }
    for (name, dir) in [
        ("sigma-kernel", repo(root, "sigma-kernel")),
        ("sigma-sh", repo(root, "sigma-sh")),
        ("sigma-sys", repo(root, "sigma-sys")),
        ("sigma-bootloader", repo(root, "sigma-bootloader")),
        ("initramfs", repo(root, "initramfs")),
    ] {
        println!("{:<38} {}", format!("repo SigmaOS-{}", name), if dir.is_dir() { "present" } else { "MISSING" });
    }
}

fn main() {
    env_logger::init();
    let cli = Cli::parse();
    let r = root(&cli.root);
    println!("root: {}", r.display());
    if let Err(e) = dispatch(cli, &r) {
        eprintln!("xtask: {}", e);
        std::process::exit(1);
    }
}

fn dispatch(cli: Cli, r: &Path) -> Result<(), String> {
    match cli.cmd {
        Cmd::BuildKernel => cmd_build_kernel(&r),
        Cmd::BuildUserspace => cmd_build_userspace(&r),
        Cmd::Initramfs { out } => cmd_initramfs(&r, &out),
        Cmd::KernelElf { out } => cmd_kernel_elf(&r, &out),
        Cmd::Qemu { kernel, initramfs, mem, timeout } => {
            cmd_qemu(&kernel, &initramfs, mem, timeout)
        }
        Cmd::QemuDebug { kernel } => {
            check_qemu()?;
            let args = vec![
                "-machine".into(),
                "q35".into(),
                "-m".into(),
                "512M".into(),
                "-kernel".into(),
                kernel.display().to_string(),
                "-serial".into(),
                "stdio".into(),
                "-display".into(),
                "none".into(),
                "-s".into(),
                "-S".into(),
            ];
            println!("qemu-system-x86_64 {}", args.join(" "));
            let mut c = Proc::new("qemu-system-x86_64");
            c.args(&args);
            let st = c.status().map_err(|e| e.to_string())?;
            if !st.success() {
                return Err(format!("qemu exited {}", st));
            }
            Ok(())
        }
        Cmd::TestQemu => Err(
            "test-qemu: requires a linked kernel ELF; run kernel-elf then qemu --timeout 10"
                .into(),
        ),
        Cmd::Status => {
            cmd_status(&r);
            Ok(())
        }
    }
}

use crate::uefi::abi::EfiSimpleTextOutputProtocol;
use crate::uefi::console::{write_console, write_usize};

#[derive(Clone, Copy)]
pub struct EntryPlan<'a> {
    pub arch: &'a str,
    pub load_addr: u64,
    pub entry_point: u64,
    pub kernel_size: usize,
    pub boot_info: usize,
}

pub fn print_entry_plan(console: *mut EfiSimpleTextOutputProtocol, plan: &EntryPlan<'_>) {
    write_console(console, "entry_call_arch: ");
    write_console(console, plan.arch);
    write_console(console, "\r\n");
    write_console(console, "entry_call_target_arch: ");
    write_console(console, target_arch_name());
    write_console(console, "\r\n");
    write_console(console, "entry_call_convention: ");
    write_console(console, calling_convention_note(plan.arch));
    write_console(console, "\r\n");
    write_console(console, "entry_call_load_addr: 0x");
    write_hex_u64(console, plan.load_addr);
    write_console(console, "\r\n");
    write_console(console, "entry_call_entry_point: 0x");
    write_hex_u64(console, plan.entry_point);
    write_console(console, "\r\n");
    write_console(console, "entry_call_kernel_size: ");
    write_usize(console, plan.kernel_size);
    write_console(console, "\r\n");
    write_console(console, "entry_call_boot_info: 0x");
    write_hex_usize(console, plan.boot_info);
    write_console(console, "\r\n");
}

pub fn target_matches_manifest(manifest_arch: &str) -> bool {
    target_arch_name() == manifest_arch
}

#[allow(dead_code)]
pub unsafe fn call_entry_point(plan: &EntryPlan<'_>) -> ! {
    unsafe { call_entry(plan.entry_point, plan.boot_info) }
}

#[cfg(target_arch = "x86_64")]
#[allow(dead_code)]
unsafe fn call_entry(entry_point: u64, boot_info: usize) -> ! {
    let entry: extern "sysv64" fn(usize) -> ! =
        unsafe { core::mem::transmute(entry_point as usize) };
    entry(boot_info)
}

#[cfg(target_arch = "aarch64")]
unsafe fn call_entry(entry_point: u64, boot_info: usize) -> ! {
    let entry: extern "C" fn(usize) -> ! = unsafe { core::mem::transmute(entry_point as usize) };
    entry(boot_info)
}

#[cfg(target_arch = "riscv64")]
unsafe fn call_entry(entry_point: u64, boot_info: usize) -> ! {
    let entry: extern "C" fn(usize) -> ! = unsafe { core::mem::transmute(entry_point as usize) };
    entry(boot_info)
}

#[cfg(not(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "riscv64"
)))]
unsafe fn call_entry(_entry_point: u64, _boot_info: usize) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

fn calling_convention_note(manifest_arch: &str) -> &'static str {
    match (target_arch_name(), manifest_arch) {
        ("x86_64", "x86_64") => "x86_64 extern sysv64 boot-info entry",
        ("aarch64", "aarch64") => "aarch64 extern C boot-info entry",
        ("riscv64", "riscv64") => "riscv64 extern C boot-info entry",
        _ => "target/manifest arch mismatch or unvalidated entry ABI",
    }
}

fn target_arch_name() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64"
    }
    #[cfg(target_arch = "riscv64")]
    {
        "riscv64"
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        "unknown"
    }
}

fn write_hex_u64(console: *mut EfiSimpleTextOutputProtocol, value: u64) {
    let mut output = [0u8; 16];
    let mut shift = 60u32;
    for byte in &mut output {
        let digit = ((value >> shift) & 0xf) as u8;
        *byte = match digit {
            0..=9 => b'0' + digit,
            _ => b'a' + (digit - 10),
        };
        shift = shift.saturating_sub(4);
    }
    let text = core::str::from_utf8(&output).unwrap_or("????????????????");
    write_console(console, text);
}

fn write_hex_usize(console: *mut EfiSimpleTextOutputProtocol, value: usize) {
    write_hex_u64(console, value as u64);
}

use crate::uefi::abi::EfiSimpleTextOutputProtocol;
#[cfg(target_arch = "x86_64")]
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_arch = "x86_64")]
const COM1_PORT: u16 = 0x3f8;
#[cfg(target_arch = "x86_64")]
const UART_RBR_THR: u16 = 0;
#[cfg(target_arch = "x86_64")]
const UART_IER: u16 = 1;
#[cfg(target_arch = "x86_64")]
const UART_FCR: u16 = 2;
#[cfg(target_arch = "x86_64")]
const UART_LCR: u16 = 3;
#[cfg(target_arch = "x86_64")]
const UART_MCR: u16 = 4;
#[cfg(target_arch = "x86_64")]
const UART_LSR: u16 = 5;
#[cfg(target_arch = "x86_64")]
const UART_DLL: u16 = 0;
#[cfg(target_arch = "x86_64")]
const UART_DLM: u16 = 1;
#[cfg(target_arch = "x86_64")]
const UART_LSR_THRE: u8 = 1 << 5;
#[cfg(target_arch = "x86_64")]
const UART_LSR_TEMT: u8 = 1 << 6;
#[cfg(target_arch = "x86_64")]
const UART_LCR_DLAB: u8 = 1 << 7;
#[cfg(target_arch = "x86_64")]
static COM1_INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn write_console(console: *mut EfiSimpleTextOutputProtocol, message: &str) {
    write_serial(message.as_bytes());

    let Some(console_ref) = (unsafe { console.as_mut() }) else {
        return;
    };

    let mut buffer = [0u16; 192];
    let mut index = 0;
    for unit in message.encode_utf16() {
        if index + 1 >= buffer.len() {
            break;
        }
        buffer[index] = unit;
        index += 1;
    }
    buffer[index] = 0;

    (console_ref.output_string)(console, buffer.as_ptr());
}

pub fn set_progress_cursor_visible(console: *mut EfiSimpleTextOutputProtocol, visible: bool) {
    write_serial(if visible { b"\x1b[?25h" } else { b"\x1b[?25l" });

    let Some(console_ref) = (unsafe { console.as_mut() }) else {
        return;
    };
    let _ = (console_ref.enable_cursor)(console, u8::from(visible));
}

#[cfg(target_arch = "x86_64")]
fn write_serial(bytes: &[u8]) {
    init_com1();
    for byte in bytes {
        if *byte == b'\n' {
            serial_putc(b'\r');
        }
        serial_putc(*byte);
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn write_serial(_bytes: &[u8]) {}

#[cfg(target_arch = "x86_64")]
fn init_com1() {
    if COM1_INITIALIZED.swap(true, Ordering::AcqRel) {
        return;
    }
    unsafe {
        outb(COM1_PORT + UART_IER, 0x00);
        outb(COM1_PORT + UART_LCR, UART_LCR_DLAB);
        outb(COM1_PORT + UART_DLL, 0x01);
        outb(COM1_PORT + UART_DLM, 0x00);
        outb(COM1_PORT + UART_LCR, 0x03);
        outb(COM1_PORT + UART_FCR, 0xc7);
        outb(COM1_PORT + UART_MCR, 0x0b);
    }
}

#[cfg(target_arch = "x86_64")]
fn serial_putc(byte: u8) {
    for _ in 0..100_000 {
        if unsafe { inb(COM1_PORT + UART_LSR) } & UART_LSR_THRE != 0 {
            unsafe { outb(COM1_PORT + UART_RBR_THR, byte) };
            wait_serial_empty();
            return;
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn wait_serial_empty() {
    for _ in 0..100_000 {
        if unsafe { inb(COM1_PORT + UART_LSR) } & UART_LSR_TEMT != 0 {
            return;
        }
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn outb(port: u16, value: u8) {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack, preserves_flags)
        );
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        core::arch::asm!(
            "in al, dx",
            in("dx") port,
            out("al") value,
            options(nomem, nostack, preserves_flags)
        );
    }
    value
}

pub fn write_usize(console: *mut EfiSimpleTextOutputProtocol, mut value: usize) {
    let mut digits = [0u8; 20];
    let mut len = 0;

    if value == 0 {
        write_console(console, "0");
        return;
    }

    while value > 0 && len < digits.len() {
        digits[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }

    let mut output = [0u8; 20];
    for index in 0..len {
        output[index] = digits[len - index - 1];
    }
    let text = core::str::from_utf8(&output[..len]).unwrap_or("?");
    write_console(console, text);
}

pub fn write_hex_u64(console: *mut EfiSimpleTextOutputProtocol, value: u64) {
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

pub fn write_utf16_nul(input: &str, output: &mut [u16]) -> Result<*mut u16, ()> {
    let mut index = 0;
    for unit in input.encode_utf16() {
        if index + 1 >= output.len() {
            return Err(());
        }
        output[index] = unit;
        index += 1;
    }
    output[index] = 0;
    Ok(output.as_mut_ptr())
}

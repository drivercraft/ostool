use crate::uefi::abi::{EfiSimpleTextOutputProtocol, EfiStatus};

pub fn write_console(console: *mut EfiSimpleTextOutputProtocol, message: &str) {
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

pub fn write_status(console: *mut EfiSimpleTextOutputProtocol, status: EfiStatus) {
    write_console(console, "0x");
    write_hex_usize(console, status.0);
}

pub fn write_utf16_nul<'a>(input: &str, output: &'a mut [u16]) -> Result<*mut u16, ()> {
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

fn write_hex_usize(console: *mut EfiSimpleTextOutputProtocol, mut value: usize) {
    let mut digits = [0u8; 16];
    let mut len = 0;

    if value == 0 {
        write_console(console, "0");
        return;
    }

    while value > 0 && len < digits.len() {
        let digit = (value & 0xf) as u8;
        digits[len] = match digit {
            0..=9 => b'0' + digit,
            _ => b'a' + (digit - 10),
        };
        value >>= 4;
        len += 1;
    }

    let mut output = [0u8; 16];
    for index in 0..len {
        output[index] = digits[len - index - 1];
    }
    let text = core::str::from_utf8(&output[..len]).unwrap_or("?");
    write_console(console, text);
}

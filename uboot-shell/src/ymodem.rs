//! YMODEM file transfer protocol implementation.
//!
//! This module implements the YMODEM protocol for file transfers over serial connections.
//! YMODEM is commonly used by U-Boot's `loady` command.
//!
//! ## Protocol Overview
//!
//! YMODEM transfers files in 128 or 1024 byte blocks with CRC16 error checking.
//! The protocol supports:
//!
//! - File name and size transmission in the first block
//! - Automatic block size selection (128 or 1024 bytes)
//! - CRC16-CCITT or checksum error detection
//! - Retry mechanism for failed transmissions

use std::io::*;

use crate::crc::crc16_ccitt;

/// Start of Header - 128 byte block
const SOH: u8 = 0x01;
/// Start of Text - 1024 byte block
const STX: u8 = 0x02;
/// End of Transmission
const EOT: u8 = 0x04;
/// Acknowledge
const ACK: u8 = 0x06;
/// Negative Acknowledge
const NAK: u8 = 0x15;
// const CAN: u8 = 0x18; // Cancel
/// End of File padding character
const EOF: u8 = 0x1A;
/// CRC mode request character
const CRC: u8 = 0x43;

/// YMODEM protocol handler for file transfers.
///
/// Implements the YMODEM protocol for sending files over serial connections.
/// Supports both CRC16 and checksum modes.
pub struct Ymodem {
    /// Whether to use CRC16 mode (true) or checksum mode (false)
    crc_mode: bool,
    /// Current block number
    blk: u8,
    /// Number of remaining retry attempts
    retries: usize,
}

impl Ymodem {
    /// Creates a new YMODEM sender.
    ///
    /// # Arguments
    ///
    /// * `crc_mode` - Whether to start in CRC16 mode (`true`) or checksum mode (`false`)
    pub fn new(crc_mode: bool) -> Self {
        Self {
            crc_mode,
            blk: 0,
            retries: 10,
        }
    }

    fn nak(&self) -> u8 {
        if self.crc_mode { CRC } else { NAK }
    }

    fn getc<D: Read>(&mut self, dev: &mut D) -> Result<u8> {
        let mut buff = [0u8; 1];
        dev.read_exact(&mut buff)?;
        Ok(buff[0])
    }

    fn wait_for_start<D: Read>(&mut self, dev: &mut D) -> Result<()> {
        loop {
            match self.getc(dev)? {
                NAK => {
                    self.crc_mode = false;
                    return Ok(());
                }
                CRC => {
                    self.crc_mode = true;
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    /// Sends a file over the YMODEM protocol.
    ///
    /// # Arguments
    ///
    /// * `dev` - The device implementing `Read + Write` (serial stream)
    /// * `file` - The readable file stream
    /// * `name` - File name reported to the receiver
    /// * `size` - File size in bytes
    /// * `on_progress` - Callback invoked with the total bytes sent so far
    ///
    /// # Errors
    ///
    /// Returns any I/O error from the underlying device or file stream.
    pub fn send<D: Write + Read, F: Read>(
        &mut self,
        dev: &mut D,
        file: &mut F,
        name: &str,
        size: usize,
        on_progress: impl Fn(usize),
    ) -> Result<()> {
        info!("Sending file: {name}");

        self.send_header(dev, name, size)?;

        let mut buff = [0u8; 1024];
        let mut send_size = 0;

        while let Ok(n) = file.read(&mut buff) {
            if n == 0 {
                break;
            }
            self.send_blk(dev, &buff[..n], EOF, false)?;
            send_size += n;
            on_progress(send_size);
        }

        dev.write_all(&[EOT])?;
        dev.flush()?;
        self.wait_ack(dev)?;

        self.send_blk(dev, &[0], 0, true)?;

        self.wait_for_start(dev)?;
        Ok(())
    }

    fn wait_ack<D: Read>(&mut self, dev: &mut D) -> Result<()> {
        let nak = self.nak();
        loop {
            let c = self.getc(dev)?;
            match c {
                ACK => return Ok(()),
                _ => {
                    if c == nak {
                        return Err(Error::new(ErrorKind::BrokenPipe, "NAK"));
                    }
                    stdout().write_all(&[c])?;
                }
            }
        }
    }

    fn send_header<D: Write + Read>(&mut self, dev: &mut D, name: &str, size: usize) -> Result<()> {
        let mut buff = Vec::new();

        buff.append(&mut name.as_bytes().to_vec());

        buff.push(0);

        buff.append(&mut format!("{}", size).as_bytes().to_vec());

        buff.push(0);

        self.send_blk(dev, &buff, 0, false)
    }

    fn send_blk<D: Write + Read>(
        &mut self,
        dev: &mut D,
        data: &[u8],
        pad: u8,
        last: bool,
    ) -> Result<()> {
        let len;
        let p;

        if data.len() > 128 {
            len = 1024;
            p = STX;
        } else {
            len = 128;
            p = SOH;
        }
        let blk = if last { 0 } else { self.blk };
        let mut err = None;
        loop {
            if self.retries == 0 {
                return Err(err.unwrap_or(Error::new(ErrorKind::BrokenPipe, "retry too much")));
            }

            dev.write_all(&[p, blk, !blk])?;

            let mut buf = vec![pad; len];
            buf[..data.len()].copy_from_slice(data);

            dev.write_all(&buf)?;

            if self.crc_mode {
                let chsum = crc16_ccitt(0, &buf);
                let crc1 = (chsum >> 8) as u8;
                let crc2 = (chsum & 0xff) as u8;

                dev.write_all(&[crc1, crc2])?;
            }
            dev.flush()?;

            match self.wait_ack(dev) {
                Ok(_) => break,
                Err(e) => {
                    err = Some(e);
                    self.retries -= 1;
                }
            }
        }

        if self.blk == u8::MAX {
            self.blk = 0;
        } else {
            self.blk += 1;
        }

        Ok(())
    }
}

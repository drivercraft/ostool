//! # uboot-shell
//!
//! A Rust library for communicating with U-Boot bootloader over serial connection.
//!
//! This crate provides functionality to interact with U-Boot shell, execute commands,
//! transfer files via YMODEM protocol, and manage environment variables.
//!
//! ## Features
//!
//! - Automatic U-Boot shell detection and synchronization
//! - Command execution with retry support
//! - YMODEM file transfer protocol implementation
//! - Environment variable management
//! - CRC16-CCITT checksum support
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use uboot_shell::UbootShell;
//! use std::io::{Read, Write};
//!
//! // Open serial port (using serialport crate)
//! let port = serialport::new("/dev/ttyUSB0", 115200)
//!     .open()
//!     .unwrap();
//! let rx = port.try_clone().unwrap();
//! let tx = port;
//!
//! // Create U-Boot shell instance (blocks until shell is ready)
//! let mut uboot = UbootShell::new(tx, rx).unwrap();
//!
//! // Execute commands
//! let output = uboot.cmd("help").unwrap();
//! println!("{}", output);
//!
//! // Get/set environment variables
//! let bootargs = uboot.env("bootargs").unwrap();
//! uboot.set_env("myvar", "myvalue").unwrap();
//!
//! // Transfer file via YMODEM
//! uboot.loady(0x80000000, "kernel.bin", |sent, total| {
//!     println!("Progress: {}/{}", sent, total);
//! }).unwrap();
//! ```
//!
//! ## Modules
//!
//! - [`crc`] - CRC16-CCITT checksum implementation
//! - [`ymodem`] - YMODEM file transfer protocol

#[macro_use]
extern crate log;

use std::{
    fs::File,
    io::*,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// CRC16-CCITT checksum implementation.
pub mod crc;

/// YMODEM file transfer protocol implementation.
pub mod ymodem;

macro_rules! dbg {
    ($($arg:tt)*) => {{
        debug!("$ {}", &std::fmt::format(format_args!($($arg)*)));
    }};
}

const CTRL_C: u8 = 0x03;
const INT_STR: &str = "<INTERRUPT>";
const INT: &[u8] = INT_STR.as_bytes();

/// U-Boot shell communication interface.
///
/// `UbootShell` provides methods to interact with U-Boot bootloader
/// over a serial connection. It handles shell synchronization,
/// command execution, and file transfers.
///
/// # Example
///
/// ```rust,no_run
/// use uboot_shell::UbootShell;
///
/// // Assuming tx and rx are Read/Write implementations
/// # fn example(tx: impl std::io::Write + Send + 'static, rx: impl std::io::Read + Send + 'static) {
/// let mut shell = UbootShell::new(tx, rx).unwrap();
/// let result = shell.cmd("printenv").unwrap();
/// # }
/// ```
pub struct UbootShell {
    /// Transmit channel for sending data to U-Boot.
    pub tx: Option<Box<dyn Write + Send>>,
    /// Receive channel for reading data from U-Boot.
    pub rx: Option<Box<dyn Read + Send>>,
    /// Shell prompt prefix detected during initialization.
    perfix: String,
}

impl UbootShell {
    /// Creates a new UbootShell instance and waits for U-Boot shell to be ready.
    ///
    /// This function will block until it successfully detects the U-Boot shell prompt.
    /// It sends interrupt signals (Ctrl+C) to ensure the shell is in a clean state.
    ///
    /// # Arguments
    ///
    /// * `tx` - A writable stream for sending data to U-Boot
    /// * `rx` - A readable stream for receiving data from U-Boot
    ///
    /// # Returns
    ///
    /// Returns `Ok(UbootShell)` if the shell is successfully initialized,
    /// or an `Err` if communication fails.
    ///
    /// # Errors
    ///
    /// Returns an error if the serial I/O fails or the prompt cannot be detected
    /// within the internal retry loop.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use uboot_shell::UbootShell;
    ///
    /// let port = serialport::new("/dev/ttyUSB0", 115200).open().unwrap();
    /// let rx = port.try_clone().unwrap();
    /// let mut uboot = UbootShell::new(port, rx).unwrap();
    /// ```
    pub fn new(tx: impl Write + Send + 'static, rx: impl Read + Send + 'static) -> Result<Self> {
        let mut s = Self {
            tx: Some(Box::new(tx)),
            rx: Some(Box::new(rx)),
            perfix: "".to_string(),
        };
        s.wait_for_shell()?;
        debug!("shell ready, perfix: `{}`", s.perfix);
        Ok(s)
    }

    fn rx(&mut self) -> &mut Box<dyn Read + Send> {
        self.rx.as_mut().unwrap()
    }

    fn tx(&mut self) -> &mut Box<dyn Write + Send> {
        self.tx.as_mut().unwrap()
    }

    fn wait_for_interrupt(&mut self) -> Result<Vec<u8>> {
        let mut tx = self.tx.take().unwrap();

        let ok = Arc::new(AtomicBool::new(false));

        let tx_handle = thread::spawn({
            let ok = ok.clone();
            move || {
                while !ok.load(Ordering::Acquire) {
                    let _ = tx.write_all(&[CTRL_C]);
                    thread::sleep(Duration::from_millis(20));
                }
                tx
            }
        });
        let mut history: Vec<u8> = Vec::new();
        let mut interrupt_line: Vec<u8> = Vec::new();
        debug!("wait for interrupt");
        loop {
            match self.read_byte() {
                Ok(ch) => {
                    history.push(ch);

                    if history.last() == Some(&b'\n') {
                        let line = history.trim_ascii_end();
                        dbg!("{}", String::from_utf8_lossy(line));
                        let it = line.ends_with(INT);
                        if it {
                            interrupt_line.extend_from_slice(line);
                        }
                        history.clear();
                        if it {
                            ok.store(true, Ordering::Release);
                            break;
                        }
                    }
                }

                Err(ref e) if e.kind() == ErrorKind::TimedOut => {
                    continue;
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }

        self.tx = Some(tx_handle.join().unwrap());

        Ok(interrupt_line)
    }

    fn clear_shell(&mut self) -> Result<()> {
        let _ = self.read_to_end(&mut vec![]);
        Ok(())
    }

    fn wait_for_shell(&mut self) -> Result<()> {
        let mut line = self.wait_for_interrupt()?;
        debug!("got {}", String::from_utf8_lossy(&line));
        line.resize(line.len() - INT.len(), 0);
        self.perfix = String::from_utf8_lossy(&line).to_string();
        self.clear_shell()?;
        Ok(())
    }

    fn read_byte(&mut self) -> Result<u8> {
        let mut buff = [0u8; 1];
        let time_out = Duration::from_secs(5);
        let start = Instant::now();

        loop {
            match self.rx().read_exact(&mut buff) {
                Ok(_) => return Ok(buff[0]),
                Err(e) => {
                    if e.kind() == ErrorKind::TimedOut {
                        if start.elapsed() > time_out {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "Timeout",
                            ));
                        }
                    } else {
                        return Err(e);
                    }
                }
            }
        }
    }

    /// Waits for a specific string to appear in the U-Boot output.
    ///
    /// Reads from the serial connection until the specified string is found.
    ///
    /// # Arguments
    ///
    /// * `val` - The string to wait for
    ///
    /// # Returns
    ///
    /// Returns the accumulated output up to and including the matched string.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying read operation times out or fails.
    pub fn wait_for_reply(&mut self, val: &str) -> Result<String> {
        let mut reply = Vec::new();
        let mut display = Vec::new();
        debug!("wait for `{}`", val);
        loop {
            let byte = self.read_byte()?;
            reply.push(byte);
            display.push(byte);
            if byte == b'\n' {
                dbg!("{}", String::from_utf8_lossy(&display).trim_end());
                display.clear();
            }

            if reply.ends_with(val.as_bytes()) {
                dbg!("{}", String::from_utf8_lossy(&display).trim_end());
                break;
            }
        }
        Ok(String::from_utf8_lossy(&reply)
            .trim()
            .trim_end_matches(&self.perfix)
            .to_string())
    }

    /// Sends a command to U-Boot without waiting for the response.
    ///
    /// This is useful for commands that don't produce output or when
    /// you want to handle the response separately.
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command string to send
    ///
    /// # Errors
    ///
    /// Returns any I/O error that occurs while writing to the serial stream.
    pub fn cmd_without_reply(&mut self, cmd: &str) -> Result<()> {
        self.tx().write_all(cmd.as_bytes())?;
        self.tx().write_all("\n".as_bytes())?;
        // self.tx().flush()?;
        // self.wait_for_reply(cmd)?;
        // debug!("cmd ok");
        Ok(())
    }

    fn _cmd(&mut self, cmd: &str) -> Result<String> {
        let _ = self.read_to_end(&mut vec![]);
        let ok_str = "cmd-ok";
        let cmd_with_id = format!("{cmd}&& echo {ok_str}");
        self.cmd_without_reply(&cmd_with_id)?;
        let perfix = self.perfix.clone();
        let res = self
            .wait_for_reply(&perfix)?
            .trim_end()
            .trim_end_matches(self.perfix.as_str().trim())
            .trim_end()
            .to_string();
        if res.ends_with(ok_str) {
            let res = res
                .trim()
                .trim_end_matches(ok_str)
                .trim_end()
                .trim_start_matches(&cmd_with_id)
                .trim()
                .to_string();
            Ok(res)
        } else {
            Err(Error::other(format!(
                "command `{cmd}` failed, response: {res}",
            )))
        }
    }

    /// Executes a command in U-Boot shell and returns the output.
    ///
    /// This method sends the command, waits for completion, and verifies
    /// that the command executed successfully. It includes automatic retry
    /// logic (up to 3 attempts) for improved reliability.
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command string to execute
    ///
    /// # Returns
    ///
    /// Returns `Ok(String)` with the command output on success,
    /// or an `Err` if the command fails after all retries.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails after retries or if serial I/O fails.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use uboot_shell::UbootShell;
    /// # fn example(uboot: &mut UbootShell) {
    /// let output = uboot.cmd("printenv bootargs").unwrap();
    /// println!("bootargs: {}", output);
    /// # }
    /// ```
    pub fn cmd(&mut self, cmd: &str) -> Result<String> {
        info!("cmd: {cmd}");
        let mut retry = 3;
        while retry > 0 {
            match self._cmd(cmd) {
                Ok(res) => return Ok(res),
                Err(e) => {
                    warn!("cmd `{}` failed: {}, retrying...", cmd, e);
                    retry -= 1;
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
        Err(Error::other(format!(
            "command `{cmd}` failed after retries",
        )))
    }

    /// Sets a U-Boot environment variable.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the environment variable
    /// * `value` - The value to set
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use uboot_shell::UbootShell;
    /// # fn example(uboot: &mut UbootShell) {
    /// uboot.set_env("bootargs", "console=ttyS0,115200").unwrap();
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns any error from the underlying command execution.
    pub fn set_env(&mut self, name: impl Into<String>, value: impl Into<String>) -> Result<()> {
        self.cmd(&format!("setenv {} {}", name.into(), value.into()))?;
        Ok(())
    }

    /// Gets the value of a U-Boot environment variable.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the environment variable
    ///
    /// # Returns
    ///
    /// Returns `Ok(String)` with the variable value, or an `Err` if not found.
    ///
    /// # Errors
    ///
    /// Returns `ErrorKind::NotFound` if the variable is not set or cannot be read.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use uboot_shell::UbootShell;
    /// # fn example(uboot: &mut UbootShell) {
    /// let bootargs = uboot.env("bootargs").unwrap();
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns `ErrorKind::NotFound` if the variable is not set or cannot be read.
    pub fn env(&mut self, name: impl Into<String>) -> Result<String> {
        let name = name.into();
        let s = self.cmd(&format!("echo ${}", name))?;
        let sp = s
            .split("\n")
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>();
        let s = sp
            .last()
            .ok_or(Error::new(
                ErrorKind::NotFound,
                format!("env {} not found", name),
            ))?
            .to_string();
        Ok(s)
    }

    /// Gets a U-Boot environment variable as an integer.
    ///
    /// Supports both decimal and hexadecimal (0x prefix) formats.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the environment variable
    ///
    /// # Returns
    ///
    /// Returns `Ok(usize)` with the parsed integer value,
    /// or an `Err` if not found or not a valid number.
    ///
    /// # Errors
    ///
    /// Returns `ErrorKind::InvalidData` if the value is not a valid integer.
    pub fn env_int(&mut self, name: impl Into<String>) -> Result<usize> {
        let name = name.into();
        let line = self.env(&name)?;
        debug!("env {name} = {line}");

        parse_int(&line).ok_or(Error::new(
            ErrorKind::InvalidData,
            format!("env {name} is not a number"),
        ))
    }

    /// Transfers a file to U-Boot memory using YMODEM protocol.
    ///
    /// Uses the U-Boot `loady` command to receive files via YMODEM protocol.
    /// The file will be loaded to the specified memory address.
    ///
    /// # Arguments
    ///
    /// * `addr` - The memory address where the file will be loaded
    /// * `file` - Path to the file to transfer
    /// * `on_progress` - Callback function called with (bytes_sent, total_bytes)
    ///
    /// # Returns
    ///
    /// Returns `Ok(String)` with the U-Boot response on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened, the path has a non-UTF-8
    /// file name, or if the serial transfer fails.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use uboot_shell::UbootShell;
    /// # fn example(uboot: &mut UbootShell) {
    /// uboot.loady(0x80000000, "kernel.bin", |sent, total| {
    ///     println!("Progress: {}/{} bytes", sent, total);
    /// }).unwrap();
    /// # }
    /// ```
    pub fn loady(
        &mut self,
        addr: usize,
        file: impl Into<PathBuf>,
        on_progress: impl Fn(usize, usize),
    ) -> Result<String> {
        self.cmd_without_reply(&format!("loady {:#x}", addr,))?;
        let crc = self.wait_for_load_crc()?;
        let mut p = ymodem::Ymodem::new(crc);

        let file = file.into();
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "file name must be valid UTF-8"))?;

        let mut file = File::open(&file)?;

        let size = file.metadata()?.len() as usize;

        p.send(self, &mut file, name, size, |p| {
            on_progress(p, size);
        })?;
        let perfix = self.perfix.clone();
        self.wait_for_reply(&perfix)
    }

    fn wait_for_load_crc(&mut self) -> Result<bool> {
        let mut reply = Vec::new();
        loop {
            let byte = self.read_byte()?;
            reply.push(byte);
            print_raw(&[byte]);

            if reply.ends_with(b"C") {
                return Ok(true);
            }
            let res = String::from_utf8_lossy(&reply);
            if res.contains("try 'help'") {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("U-Boot loady failed: {res}"),
                ));
            }
        }
    }
}

impl Read for UbootShell {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.rx().read(buf)
    }
}

impl Write for UbootShell {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.tx().write(buf)
    }

    fn flush(&mut self) -> Result<()> {
        self.tx().flush()
    }
}

fn parse_int(line: &str) -> Option<usize> {
    let mut line = line.trim();
    let mut radix = 10;
    if line.starts_with("0x") {
        line = &line[2..];
        radix = 16;
    }
    u64::from_str_radix(line, radix).ok().map(|o| o as _)
}

fn print_raw(buff: &[u8]) {
    #[cfg(target_os = "windows")]
    print_raw_win(buff);
    #[cfg(not(target_os = "windows"))]
    stdout().write_all(buff).unwrap();
}

#[cfg(target_os = "windows")]
fn print_raw_win(buff: &[u8]) {
    use std::sync::Mutex;
    static PRINT_BUFF: Mutex<Vec<u8>> = Mutex::new(Vec::new());

    let mut g = PRINT_BUFF.lock().unwrap();

    g.extend_from_slice(buff);

    if g.ends_with(b"\n") {
        let s = String::from_utf8_lossy(&g[..]);
        println!("{}", s.trim());
        g.clear();
    }
}

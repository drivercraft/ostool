//! Async U-Boot shell communication over runtime-neutral futures I/O.

#[macro_use]
extern crate log;

use std::{
    io::{Error, ErrorKind, Result, stdout},
    path::PathBuf,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{
    AsyncReadExt, AsyncWriteExt,
    future::{Either, FutureExt, select},
    io::{AllowStdIo, AsyncRead, AsyncWrite},
    pin_mut,
};
use futures_timer::Delay;

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

type Tx = Box<dyn AsyncWrite + Send + Unpin>;
type Rx = Box<dyn AsyncRead + Send + Unpin>;

pub struct UbootShell {
    /// Transmit stream for sending bytes to U-Boot.
    pub tx: Option<Tx>,
    /// Receive stream for reading bytes from U-Boot.
    pub rx: Option<Rx>,
    /// Shell prompt prefix detected during initialization.
    perfix: String,
}

impl UbootShell {
    pub async fn new(
        tx: impl AsyncWrite + Send + Unpin + 'static,
        rx: impl AsyncRead + Send + Unpin + 'static,
    ) -> Result<Self> {
        let mut shell = Self {
            tx: Some(Box::new(tx)),
            rx: Some(Box::new(rx)),
            perfix: String::new(),
        };
        shell.wait_for_shell().await?;
        debug!("shell ready, perfix: `{}`", shell.perfix);
        Ok(shell)
    }

    fn rx(&mut self) -> &mut Rx {
        self.rx.as_mut().unwrap()
    }

    fn tx(&mut self) -> &mut Tx {
        self.tx.as_mut().unwrap()
    }

    async fn wait_for_interrupt(&mut self) -> Result<Vec<u8>> {
        let mut history = Vec::new();
        let mut interrupt_line = Vec::new();
        let interval = Duration::from_millis(20);
        let mut last_interrupt = std::time::Instant::now() - interval;

        debug!("wait for interrupt");
        loop {
            if last_interrupt.elapsed() >= interval {
                self.tx().write_all(&[CTRL_C]).await?;
                self.tx().flush().await?;
                last_interrupt = std::time::Instant::now();
            }

            match self.read_byte_with_timeout(interval).await {
                Ok(ch) => {
                    history.push(ch);
                    if history.last() == Some(&b'\n') {
                        let line = history.trim_ascii_end();
                        dbg!("{}", String::from_utf8_lossy(line));
                        let interrupted = line.ends_with(INT);
                        if interrupted {
                            interrupt_line.extend_from_slice(line);
                        }
                        history.clear();
                        if interrupted {
                            break;
                        }
                    }
                }
                Err(err) if err.kind() == ErrorKind::TimedOut => {}
                Err(err) => return Err(err),
            }
        }

        Ok(interrupt_line)
    }

    async fn clear_shell(&mut self) -> Result<()> {
        loop {
            match self
                .read_byte_with_timeout(Duration::from_millis(300))
                .await
            {
                Ok(_) => {}
                Err(err) if err.kind() == ErrorKind::TimedOut => return Ok(()),
                Err(err) => return Err(err),
            }
        }
    }

    async fn wait_for_shell(&mut self) -> Result<()> {
        let mut line = self.wait_for_interrupt().await?;
        debug!("got {}", String::from_utf8_lossy(&line));
        line.resize(line.len().saturating_sub(INT.len()), 0);
        self.perfix = String::from_utf8_lossy(&line).to_string();
        self.clear_shell().await?;
        Ok(())
    }

    async fn read_byte(&mut self) -> Result<u8> {
        self.read_byte_with_timeout(Duration::from_secs(5)).await
    }

    async fn read_byte_with_timeout(&mut self, timeout_limit: Duration) -> Result<u8> {
        let mut buff = [0u8; 1];
        let start = std::time::Instant::now();

        loop {
            let read = self.rx().read_exact(&mut buff).fuse();
            let delay = Delay::new(Duration::from_millis(200)).fuse();
            pin_mut!(read, delay);

            match select(read, delay).await {
                Either::Left((Ok(_), _)) => return Ok(buff[0]),
                Either::Left((Err(err), _)) => return Err(err),
                Either::Right((_, _)) => {
                    if start.elapsed() > timeout_limit {
                        return Err(Error::new(ErrorKind::TimedOut, "Timeout"));
                    }
                }
            }
        }
    }

    pub async fn wait_for_reply(&mut self, val: &str) -> Result<String> {
        let mut reply = Vec::new();
        let mut display = Vec::new();
        debug!("wait for `{}`", val);

        loop {
            let byte = self.read_byte().await?;
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

    pub async fn cmd_without_reply(&mut self, cmd: &str) -> Result<()> {
        self.tx().write_all(cmd.as_bytes()).await?;
        self.tx().write_all(b"\n").await?;
        self.tx().flush().await?;
        Ok(())
    }

    async fn _cmd(&mut self, cmd: &str) -> Result<String> {
        self.clear_shell().await?;
        let ok_str = "cmd-ok";
        let cmd_with_id = format!("{cmd}&& echo {ok_str}");
        self.cmd_without_reply(&cmd_with_id).await?;
        let perfix = self.perfix.clone();
        let res = self
            .wait_for_reply(&perfix)
            .await?
            .trim_end()
            .trim_end_matches(self.perfix.as_str().trim())
            .trim_end()
            .to_string();

        if res.ends_with(ok_str) {
            Ok(res
                .trim()
                .trim_end_matches(ok_str)
                .trim_end()
                .trim_start_matches(&cmd_with_id)
                .trim()
                .to_string())
        } else {
            Err(Error::other(format!(
                "command `{cmd}` failed, response: {res}",
            )))
        }
    }

    pub async fn cmd(&mut self, cmd: &str) -> Result<String> {
        info!("cmd: {cmd}");
        let mut retry = 3;
        while retry > 0 {
            match self._cmd(cmd).await {
                Ok(res) => return Ok(res),
                Err(err) => {
                    warn!("cmd `{}` failed: {}, retrying...", cmd, err);
                    retry -= 1;
                    Delay::new(Duration::from_millis(100)).await;
                }
            }
        }
        Err(Error::other(format!(
            "command `{cmd}` failed after retries",
        )))
    }

    pub async fn set_env(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<()> {
        self.cmd(&format!("setenv {} {}", name.into(), value.into()))
            .await?;
        Ok(())
    }

    pub async fn env(&mut self, name: impl Into<String>) -> Result<String> {
        let name = name.into();
        let s = self.cmd(&format!("echo ${name}")).await?;
        let parts = s
            .split('\n')
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        let value = parts
            .last()
            .ok_or(Error::new(
                ErrorKind::NotFound,
                format!("env {name} not found"),
            ))?
            .to_string();
        Ok(value)
    }

    pub async fn env_int(&mut self, name: impl Into<String>) -> Result<usize> {
        let name = name.into();
        let line = self.env(&name).await?;
        debug!("env {name} = {line}");

        parse_int(&line).ok_or(Error::new(
            ErrorKind::InvalidData,
            format!("env {name} is not a number"),
        ))
    }

    pub async fn loady(
        &mut self,
        addr: usize,
        file: impl Into<PathBuf>,
        on_progress: impl Fn(usize, usize),
    ) -> Result<String> {
        self.cmd_without_reply(&format!("loady {addr:#x}")).await?;
        let crc = self.wait_for_load_crc().await?;
        let mut protocol = ymodem::Ymodem::new(crc);

        let file = file.into();
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "file name must be valid UTF-8"))?;
        let size = std::fs::metadata(&file)?.len() as usize;
        let mut file = AllowStdIo::new(std::fs::File::open(&file)?);

        protocol
            .send(self, &mut file, name, size, |sent| on_progress(sent, size))
            .await?;
        let perfix = self.perfix.clone();
        self.wait_for_reply(&perfix).await
    }

    async fn wait_for_load_crc(&mut self) -> Result<bool> {
        let mut reply = Vec::new();
        loop {
            let byte = self.read_byte().await?;
            reply.push(byte);
            print_raw(&[byte]).await?;

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

impl AsyncRead for UbootShell {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        let this = self.get_mut();
        Pin::new(this.rx.as_mut().unwrap().as_mut()).poll_read(cx, buf)
    }
}

impl AsyncWrite for UbootShell {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        let this = self.get_mut();
        Pin::new(this.tx.as_mut().unwrap().as_mut()).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let this = self.get_mut();
        Pin::new(this.tx.as_mut().unwrap().as_mut()).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let this = self.get_mut();
        Pin::new(this.tx.as_mut().unwrap().as_mut()).poll_close(cx)
    }
}

fn parse_int(line: &str) -> Option<usize> {
    let mut line = line.trim();
    let mut radix = 10;
    if line.starts_with("0x") {
        line = &line[2..];
        radix = 16;
    }
    u64::from_str_radix(line, radix)
        .ok()
        .map(|value| value as usize)
}

async fn print_raw(buff: &[u8]) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        print_raw_win(buff);
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut out = AllowStdIo::new(stdout());
        out.write_all(buff).await
    }
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

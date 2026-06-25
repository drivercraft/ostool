//! Async U-Boot shell communication over runtime-neutral futures I/O.

#[macro_use]
extern crate log;

use std::{
    io::{Error, ErrorKind, Result, stdout},
    path::{Path, PathBuf},
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
const LOADY_MAX_ATTEMPTS: usize = 3;
const LOADY_RETRY_DELAY: Duration = Duration::from_millis(300);

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
        debug!("wait for `{val}`");

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
                    warn!("cmd `{cmd}` failed: {err}, retrying...");
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
        let file = file.into();

        for attempt in 1..=LOADY_MAX_ATTEMPTS {
            match self.loady_once(addr, &file, &on_progress).await {
                Ok(reply) => return Ok(reply),
                Err(err) if attempt < LOADY_MAX_ATTEMPTS => {
                    warn!(
                        "loady attempt {attempt}/{LOADY_MAX_ATTEMPTS} failed: {err}; retrying..."
                    );
                    self.wait_for_shell().await.map_err(|recover_err| {
                        Error::other(format!(
                            "loady attempt {attempt} failed and shell recovery failed: {recover_err}",
                        ))
                    })?;
                    Delay::new(LOADY_RETRY_DELAY).await;
                }
                Err(err) => {
                    return Err(Error::other(format!(
                        "loady failed after {LOADY_MAX_ATTEMPTS} attempts: {err}"
                    )));
                }
            }
        }

        unreachable!("LOADY_MAX_ATTEMPTS must be greater than zero")
    }

    async fn loady_once(
        &mut self,
        addr: usize,
        file: &Path,
        on_progress: &impl Fn(usize, usize),
    ) -> Result<String> {
        self.clear_shell().await?;
        self.cmd_without_reply(&format!("loady {addr:#x}")).await?;
        let crc = self.wait_for_load_crc().await?;
        let mut protocol = ymodem::Ymodem::new(crc);

        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "file name must be valid UTF-8"))?;
        let size = std::fs::metadata(file)?.len() as usize;
        let mut file = AllowStdIo::new(std::fs::File::open(file)?);

        on_progress(0, size);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        fs,
        sync::{Arc, Mutex},
    };

    #[derive(Default)]
    struct LoadyScript {
        reads: VecDeque<u8>,
        writes: Vec<u8>,
        command: Vec<u8>,
        loady_count: usize,
        interrupted: bool,
        accepting_commands: bool,
    }

    impl LoadyScript {
        fn queue_read(&mut self, bytes: impl AsRef<[u8]>) {
            self.reads.extend(bytes.as_ref());
        }

        fn handle_write(&mut self, bytes: &[u8]) {
            self.writes.extend_from_slice(bytes);

            if bytes == [CTRL_C] {
                self.command.clear();
                self.accepting_commands = true;
                if !self.interrupted {
                    self.interrupted = true;
                    self.queue_read(b"=> <INTERRUPT>\n");
                }
                return;
            }

            if !self.accepting_commands {
                return;
            }

            for &byte in bytes {
                self.command.push(byte);
                if byte == b'\n' {
                    let command = std::mem::take(&mut self.command);
                    if command.starts_with(b"loady ") {
                        self.loady_count += 1;
                        self.accepting_commands = false;
                        self.queue_loady_response();
                    }
                } else if self.command.len() > 256 {
                    self.command.clear();
                }
            }
        }

        fn queue_loady_response(&mut self) {
            match self.loady_count {
                1 => {
                    self.queue_read(*b"C");
                    self.queue_read([ymodem::CRC; ymodem::DEFAULT_BLOCK_RETRIES]);
                }
                2 => {
                    self.queue_read(*b"C");
                    self.queue_read([ymodem::ACK, ymodem::ACK, ymodem::ACK, ymodem::ACK, b'C']);
                    self.queue_read(b"done\n=> ");
                }
                _ => {}
            }
        }
    }

    #[derive(Clone)]
    struct ScriptedTx {
        script: Arc<Mutex<LoadyScript>>,
    }

    #[derive(Clone)]
    struct ScriptedRx {
        script: Arc<Mutex<LoadyScript>>,
    }

    impl AsyncWrite for ScriptedTx {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<Result<usize>> {
            self.script.lock().unwrap().handle_write(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncRead for ScriptedRx {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<Result<usize>> {
            let mut script = self.script.lock().unwrap();
            if script.reads.is_empty() {
                return Poll::Pending;
            }

            let n = buf.len().min(script.reads.len());
            for slot in &mut buf[..n] {
                *slot = script.reads.pop_front().unwrap();
            }
            Poll::Ready(Ok(n))
        }
    }

    #[tokio::test]
    async fn loady_restarts_transfer_after_receiver_rejects_first_attempt() -> Result<()> {
        let script = Arc::new(Mutex::new(LoadyScript::default()));
        script.lock().unwrap().accepting_commands = true;
        let mut shell = UbootShell {
            tx: Some(Box::new(ScriptedTx {
                script: script.clone(),
            })),
            rx: Some(Box::new(ScriptedRx {
                script: script.clone(),
            })),
            perfix: "=> ".to_string(),
        };

        let file =
            std::env::temp_dir().join(format!("uboot-shell-loady-retry-{}", std::process::id()));
        fs::write(&file, b"payload")?;

        let progress = Arc::new(Mutex::new(Vec::new()));
        let reply = shell
            .loady(0x80200000, file.clone(), {
                let progress = progress.clone();
                move |sent, size| progress.lock().unwrap().push((sent, size))
            })
            .await;
        let _ = fs::remove_file(&file);

        assert!(reply?.contains("done"));
        let script = script.lock().unwrap();
        let writes = String::from_utf8_lossy(&script.writes);
        assert_eq!(writes.matches("loady 0x80200000").count(), 2);
        assert!(script.writes.contains(&CTRL_C));
        assert_eq!(*progress.lock().unwrap(), vec![(0, 7), (0, 7), (7, 7)]);
        Ok(())
    }
}

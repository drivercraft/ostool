//! Async YMODEM file transfer protocol implementation.

use std::io::{Error, ErrorKind, Result};

use futures::{
    AsyncReadExt, AsyncWriteExt,
    io::{AllowStdIo, AsyncRead, AsyncWrite},
};

use crate::crc::crc16_ccitt;

const SOH: u8 = 0x01;
const STX: u8 = 0x02;
const EOT: u8 = 0x04;
pub(crate) const ACK: u8 = 0x06;
pub(crate) const NAK: u8 = 0x15;
const EOF: u8 = 0x1A;
pub(crate) const CRC: u8 = 0x43;
pub(crate) const DEFAULT_BLOCK_RETRIES: usize = 10;

pub struct Ymodem {
    crc_mode: bool,
    blk: u8,
    max_block_retries: usize,
}

impl Ymodem {
    pub fn new(crc_mode: bool) -> Self {
        Self {
            crc_mode,
            blk: 0,
            max_block_retries: DEFAULT_BLOCK_RETRIES,
        }
    }

    fn nak(&self) -> u8 {
        if self.crc_mode { CRC } else { NAK }
    }

    async fn getc<D: AsyncRead + Unpin>(&mut self, dev: &mut D) -> Result<u8> {
        let mut buff = [0u8; 1];
        dev.read_exact(&mut buff).await?;
        Ok(buff[0])
    }

    async fn wait_for_start<D: AsyncRead + Unpin>(&mut self, dev: &mut D) -> Result<()> {
        loop {
            match self.getc(dev).await? {
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

    pub async fn send<D, F>(
        &mut self,
        dev: &mut D,
        file: &mut F,
        name: &str,
        size: usize,
        on_progress: impl Fn(usize),
    ) -> Result<()>
    where
        D: AsyncWrite + AsyncRead + Unpin,
        F: AsyncRead + Unpin,
    {
        info!("Sending file: {name}");

        self.send_header(dev, name, size).await?;

        let mut buff = [0u8; 1024];
        let mut send_size = 0;

        loop {
            let n = file.read(&mut buff).await?;
            if n == 0 {
                break;
            }
            self.send_blk(dev, &buff[..n], EOF, false).await?;
            send_size += n;
            on_progress(send_size);
        }

        dev.write_all(&[EOT]).await?;
        dev.flush().await?;
        self.wait_ack(dev).await?;

        self.send_blk(dev, &[0], 0, true).await?;
        self.wait_for_start(dev).await?;
        Ok(())
    }

    async fn wait_ack<D: AsyncRead + Unpin>(&mut self, dev: &mut D) -> Result<()> {
        let nak = self.nak();
        loop {
            let c = self.getc(dev).await?;
            match c {
                ACK => return Ok(()),
                _ => {
                    if c == nak {
                        return Err(Error::new(ErrorKind::BrokenPipe, "NAK"));
                    }
                    let mut out = AllowStdIo::new(std::io::stdout());
                    out.write_all(&[c]).await?;
                }
            }
        }
    }

    async fn send_header<D: AsyncWrite + AsyncRead + Unpin>(
        &mut self,
        dev: &mut D,
        name: &str,
        size: usize,
    ) -> Result<()> {
        let mut buff = Vec::new();
        buff.append(&mut name.as_bytes().to_vec());
        buff.push(0);
        buff.append(&mut format!("{size}").as_bytes().to_vec());
        buff.push(0);
        self.send_blk(dev, &buff, 0, false).await
    }

    async fn send_blk<D: AsyncWrite + AsyncRead + Unpin>(
        &mut self,
        dev: &mut D,
        data: &[u8],
        pad: u8,
        last: bool,
    ) -> Result<()> {
        let (len, p) = if data.len() > 128 {
            (1024, STX)
        } else {
            (128, SOH)
        };
        let blk = if last { 0 } else { self.blk };
        let mut err = None;
        let mut retries = self.max_block_retries;

        loop {
            if retries == 0 {
                return Err(err.unwrap_or(Error::new(ErrorKind::BrokenPipe, "retry too much")));
            }

            dev.write_all(&[p, blk, !blk]).await?;

            let mut buf = vec![pad; len];
            buf[..data.len()].copy_from_slice(data);
            dev.write_all(&buf).await?;

            if self.crc_mode {
                let chsum = crc16_ccitt(0, &buf);
                let crc1 = (chsum >> 8) as u8;
                let crc2 = (chsum & 0xff) as u8;
                dev.write_all(&[crc1, crc2]).await?;
            }
            dev.flush().await?;

            match self.wait_ack(dev).await {
                Ok(_) => break,
                Err(e) => {
                    err = Some(e);
                    retries -= 1;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        pin::Pin,
        sync::Mutex,
        task::{Context, Poll},
    };

    use futures::io::Cursor;

    struct ScriptedDevice {
        reads: VecDeque<u8>,
        writes: Vec<u8>,
    }

    impl AsyncRead for ScriptedDevice {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<Result<usize>> {
            if self.reads.is_empty() {
                return Poll::Ready(Ok(0));
            }

            let n = buf.len().min(self.reads.len());
            for slot in &mut buf[..n] {
                *slot = self.reads.pop_front().unwrap();
            }
            Poll::Ready(Ok(n))
        }
    }

    impl AsyncWrite for ScriptedDevice {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<Result<usize>> {
            self.writes.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn acked_block_resets_retry_budget() -> Result<()> {
        let mut reads = VecDeque::from([CRC, ACK]);
        reads.extend(std::iter::repeat_n(CRC, DEFAULT_BLOCK_RETRIES - 1));
        reads.extend([ACK, ACK, ACK, CRC]);

        let mut dev = ScriptedDevice {
            reads,
            writes: Vec::new(),
        };
        let mut file = Cursor::new(b"payload".to_vec());
        let progress = Mutex::new(Vec::new());

        Ymodem::new(true)
            .send(&mut dev, &mut file, "kernel", 7, |sent| {
                progress.lock().unwrap().push(sent);
            })
            .await?;

        assert_eq!(*progress.lock().unwrap(), vec![7]);
        assert!(dev.writes.contains(&EOT));
        Ok(())
    }
}

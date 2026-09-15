//! Exclusive, cancellation-safe access to Unix terminal input.

use std::{io, os::fd::OwnedFd};

use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use tokio::io::unix::AsyncFd;

pub(super) struct Input {
    fd: AsyncFd<OwnedFd>,
    original_flags: OFlags,
}

impl Input {
    /// The terminal loop must be the sole stdin reader while this owner lives.
    pub(super) fn new() -> io::Result<Self> {
        Self::from_fd(rustix::io::dup(io::stdin())?)
    }

    fn from_fd(fd: OwnedFd) -> io::Result<Self> {
        let original_flags = fcntl_getfl(&fd)?;
        let fd = AsyncFd::new(fd)?;
        fcntl_setfl(fd.get_ref(), original_flags | OFlags::NONBLOCK)?;
        Ok(Self { fd, original_flags })
    }

    pub(super) async fn next(&mut self) -> Option<io::Result<Vec<u8>>> {
        loop {
            let mut ready = match self.fd.readable().await {
                Ok(ready) => ready,
                Err(error) => return Some(Err(error)),
            };
            let mut bytes = vec![0; 4096];
            match ready
                .try_io(|fd| rustix::io::read(fd.get_ref(), &mut bytes).map_err(io::Error::from))
            {
                Ok(Ok(0)) => return None,
                Ok(Ok(length)) => {
                    bytes.truncate(length);
                    return Some(Ok(bytes));
                }
                Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
                Ok(Err(error)) => return Some(Err(error)),
                Err(_) => continue,
            }
        }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        // dup shares status flags with stdin; restore them before releasing it.
        let _ = fcntl_setfl(self.fd.get_ref(), self.original_flags);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, os::unix::net::UnixStream, time::Duration};

    #[tokio::test]
    async fn cancelled_read_preserves_bytes_and_restores_shared_flags() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let flags = fcntl_getfl(&reader).unwrap();
        let mut input = Input::from_fd(reader.try_clone().unwrap().into()).unwrap();
        assert!(fcntl_getfl(&reader).unwrap().contains(OFlags::NONBLOCK));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), input.next())
                .await
                .is_err()
        );
        let expected = b"\x1b[24;80R\x1b[200~https://example.test/\x1b[201~";
        writer.write_all(expected).unwrap();
        let mut actual = Vec::new();
        while actual.len() < expected.len() {
            actual.extend(input.next().await.unwrap().unwrap());
        }
        assert_eq!(actual, expected);
        drop(writer);
        assert!(input.next().await.is_none());
        drop(input);
        assert_eq!(fcntl_getfl(&reader).unwrap(), flags);
    }
}

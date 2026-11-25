//! QEMU-backed smoke tests for the asynchronous U-Boot shell.

use std::{
    io::{Error, Result},
    pin::Pin,
    process::{Child, Command},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    task::{Context, Poll},
};

use futures::io::AsyncWrite;
use log::{debug, info};
use ntest::timeout;
use tokio::{
    net::TcpStream,
    time::{Duration, sleep},
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use uboot_shell::UbootShell;

static PORT: AtomicU32 = AtomicU32::new(10000);

struct QemuProcess(Child);

impl Drop for QemuProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts QEMU with the bundled U-Boot image and returns an attached shell.
async fn new_uboot() -> (QemuProcess, UbootShell) {
    new_uboot_with_flush_fault(Arc::new(AtomicBool::new(false))).await
}

// Forward the real byte stream unchanged, but lose one host-side flush result
// AFTER the newline was accepted. This cannot undo the target's side effect.
struct SubmitFlushFault<T> {
    inner: T,
    armed: Arc<AtomicBool>,
    submitted: bool,
}

impl<T: AsyncWrite + Unpin> AsyncWrite for SubmitFlushFault<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize>> {
        match Pin::new(&mut self.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(count)) => {
                if buf[..count].contains(&b'\n') && self.armed.load(Ordering::SeqCst) {
                    self.submitted = true;
                }
                Poll::Ready(Ok(count))
            }
            result => result,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        match Pin::new(&mut self.inner).poll_flush(cx) {
            Poll::Ready(Ok(())) if self.submitted && self.armed.swap(false, Ordering::SeqCst) => {
                Poll::Ready(Err(Error::other("injected post-newline flush failure")))
            }
            result => result,
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}

async fn new_uboot_with_flush_fault(armed: Arc<AtomicBool>) -> (QemuProcess, UbootShell) {
    let _ = env_logger::try_init();
    let port = PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    // qemu-system-aarch64 -machine virt -cpu cortex-a57 -nographic -bios assets/u-boot.bin
    let out = QemuProcess(
        Command::new("qemu-system-aarch64")
            .arg("-serial")
            .arg(format!("tcp::{port},server,nowait"))
            .args([
                "-machine",
                "virt",
                "-cpu",
                "cortex-a57",
                "-nographic",
                "-monitor",
                "none",
                "-net",
                "none",
                "-bios",
                "../assets/u-boot.bin",
            ])
            .spawn()
            .unwrap(),
    );

    loop {
        sleep(Duration::from_millis(100)).await;
        match TcpStream::connect(format!("127.0.0.1:{port}")).await {
            Ok(s) => {
                s.set_nodelay(true).unwrap();
                let (rx, tx) = s.into_split();
                info!("connect ok");
                return (
                    out,
                    UbootShell::new(
                        SubmitFlushFault {
                            inner: tx.compat_write(),
                            armed,
                            submitted: false,
                        },
                        rx.compat(),
                    )
                    .await
                    .unwrap(),
                );
            }
            Err(e) => {
                debug!("wait for qemu serial port ready: {e}");
            }
        }
    }
}

#[tokio::test]
#[timeout(15000)]
async fn test_shell() {
    let (_qemu, _uboot) = new_uboot().await;
    info!("test_shell ok");
}

#[tokio::test]
#[timeout(15000)]
async fn test_cmd() {
    let (_qemu, mut uboot) = new_uboot().await;
    let res = uboot.cmd("help").await.unwrap();
    println!("{}", res);
}

#[tokio::test]
#[timeout(15000)]
async fn test_setenv() {
    let (_qemu, mut uboot) = new_uboot().await;
    let cmdline = "earlycon init=/bin/sh HOME=/root USER=root HOSTNAME=starry -- -c \"cd /root; \
                   export PS1=$USER@$HOSTNAME:~#; exec /bin/sh -i\"";
    uboot
        .set_env("bootargs", format!("'{cmdline}'"))
        .await
        .unwrap();
    assert_eq!(
        uboot.cmd("printenv bootargs").await.unwrap(),
        format!("bootargs={cmdline}")
    );
}

#[tokio::test]
#[timeout(15000)]
async fn test_env() {
    let (_qemu, mut uboot) = new_uboot().await;
    uboot.set_env("fdt_addr", "0x40000000").await.unwrap();
    info!("set fdt_addr ok");
    assert_eq!(uboot.env_int("fdt_addr").await.unwrap(), 0x40000000);
}

#[tokio::test]
#[timeout(15000)]
async fn submitted_flush_failure_executes_side_effect_once() {
    let armed = Arc::new(AtomicBool::new(false));
    let (_qemu, mut uboot) = new_uboot_with_flush_fault(armed.clone()).await;
    uboot.set_env("side_count", "0").await.unwrap();
    armed.store(true, Ordering::SeqCst);
    let error = uboot
        .cmd("setexpr side_count ${side_count} + 1")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("execution uncertain"), "{error}");
    assert!(!armed.load(Ordering::SeqCst), "flush fault was not reached");
    assert!(uboot.cmd("echo forbidden").await.is_err());

    // Explicitly resolve the old response and establish a new shell session.
    // Ordinary entry points must not do either on the caller's behalf.
    uboot.wait_for_reply("=> ").await.unwrap();
    let mut reconnected = UbootShell::new(uboot.tx.take().unwrap(), uboot.rx.take().unwrap())
        .await
        .unwrap();
    assert_eq!(reconnected.env_int("side_count").await.unwrap(), 1);
    println!("QEMU post-submission fault: side_count=1");
}

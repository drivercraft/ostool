//! QEMU-backed smoke tests for the asynchronous U-Boot shell.

use futures::io::AsyncWrite;
use log::info;
use std::{
    future::Future,
    io::{Error, Result},
    path::PathBuf,
    pin::Pin,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::{
    io::AsyncReadExt,
    net::TcpListener,
    time::{Duration, sleep, timeout},
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use uboot_shell::UbootShell;

const STAGE_TIMEOUT: Duration = Duration::from_secs(15);

async fn stage<T>(name: &str, future: impl Future<Output = T>) -> T {
    timeout(STAGE_TIMEOUT, future)
        .await
        .unwrap_or_else(|_| panic!("timed out after {STAGE_TIMEOUT:?}: {name}"))
}

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
    // Keep the listener bound while QEMU starts. Connecting the chardev before
    // the guest boots also prevents early console output from being discarded.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let bios = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/u-boot.bin");

    let mut out = QemuProcess(
        Command::new("qemu-system-aarch64")
            .arg("-chardev")
            .arg(format!(
                "socket,id=serial,host={},port={},server=off,nodelay=on",
                address.ip(),
                address.port()
            ))
            .arg("-serial")
            .arg("chardev:serial")
            .args([
                "-machine",
                "virt",
                "-cpu",
                "cortex-a57",
                "-display",
                "none",
                "-monitor",
                "none",
                "-net",
                "none",
                "-bios",
            ])
            .arg(bios)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );

    let mut socket = stage("QEMU serial connection", async {
        loop {
            tokio::select! {
                connection = listener.accept() => break connection.unwrap().0,
                _ = sleep(Duration::from_millis(20)) => {
                    if let Some(status) = out.0.try_wait().unwrap() {
                        panic!("QEMU exited before connecting its serial port: {status}");
                    }
                }
            }
        }
    })
    .await;
    socket.set_nodelay(true).unwrap();
    // The socket can connect before the firmware accepts console interrupts.
    // Synchronize on the guest's boot prompt before starting Ctrl-C negotiation.
    stage("U-Boot autoboot countdown", async {
        let marker = b"Hit any key to stop autoboot:";
        let mut boot = Vec::new();
        while !boot.ends_with(marker) {
            boot.push(socket.read_u8().await.unwrap());
        }
    })
    .await;
    let (rx, tx) = socket.into_split();
    let shell = stage(
        "U-Boot shell initialization",
        UbootShell::new(
            SubmitFlushFault {
                inner: tx.compat_write(),
                armed,
                submitted: false,
            },
            rx.compat(),
        ),
    )
    .await
    .unwrap();
    (out, shell)
}

#[tokio::test]
async fn test_shell() {
    let (_qemu, _uboot) = new_uboot().await;
    info!("test_shell ok");
}

#[tokio::test]
async fn test_cmd() {
    let (_qemu, mut uboot) = new_uboot().await;
    let res = stage("help command", uboot.cmd("help")).await.unwrap();
    println!("{}", res);
}

#[tokio::test]
async fn test_setenv() {
    let (_qemu, mut uboot) = new_uboot().await;
    let cmdline = "earlycon init=/bin/sh HOME=/root USER=root HOSTNAME=starry -- -c \"cd /root; export PS1=$USER@$HOSTNAME:~#; exec /bin/sh -i\"";
    stage(
        "set bootargs",
        uboot.set_env("bootargs", format!("'{cmdline}'")),
    )
    .await
    .unwrap();
    assert_eq!(
        stage("read bootargs", uboot.cmd("printenv bootargs"))
            .await
            .unwrap(),
        format!("bootargs={cmdline}")
    );
}

#[tokio::test]
async fn test_env() {
    let (_qemu, mut uboot) = new_uboot().await;
    stage("set fdt_addr", uboot.set_env("fdt_addr", "0x40000000"))
        .await
        .unwrap();
    info!("set fdt_addr ok");
    assert_eq!(
        stage("read fdt_addr", uboot.env_int("fdt_addr"))
            .await
            .unwrap(),
        0x40000000
    );
}

#[tokio::test]
async fn submitted_flush_failure_executes_side_effect_once() {
    stage("post-submission flush failure", async {
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
    })
    .await;
}

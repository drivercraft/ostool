//! QEMU-backed smoke tests for the asynchronous U-Boot shell.

use std::{
    process::{Child, Command},
    sync::atomic::AtomicU32,
};

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
                let (rx, tx) = s.into_split();
                info!("connect ok");
                return (
                    out,
                    UbootShell::new(tx.compat_write(), rx.compat())
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
    let cmdline = "earlycon init=/bin/sh HOME=/root USER=root HOSTNAME=starry -- -c \"cd /root; export PS1=$USER@$HOSTNAME:~#; exec /bin/sh -i\"";
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

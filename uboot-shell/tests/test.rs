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

async fn new_uboot() -> (Child, UbootShell) {
    let port = PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    // qemu-system-aarch64 -machine virt -cpu cortex-a57 -nographic -bios assets/u-boot.bin
    let out = Command::new("qemu-system-aarch64")
        .arg("-serial")
        .arg(format!("tcp::{port},server,nowait"))
        .args([
            "-machine",
            "virt",
            "-cpu",
            "cortex-a57",
            "-nographic",
            "-bios",
            "../assets/u-boot.bin",
        ])
        .spawn()
        .unwrap();

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
    let (mut out, _uboot) = new_uboot().await;
    info!("test_shell ok");
    let _ = out.kill();
    out.wait().unwrap();
}

#[tokio::test]
#[timeout(15000)]
async fn test_cmd() {
    let (mut out, mut uboot) = new_uboot().await;
    let res = uboot.cmd("help").await.unwrap();
    println!("{}", res);
    let _ = out.kill();
    out.wait().unwrap();
}

#[tokio::test]
#[timeout(15000)]
async fn test_setenv() {
    let (mut out, mut uboot) = new_uboot().await;
    uboot.set_env("ipaddr", "127.0.0.1").await.unwrap();
    let _ = out.kill();
    out.wait().unwrap();
}

#[tokio::test]
#[timeout(15000)]
async fn test_env() {
    let (mut out, mut uboot) = new_uboot().await;
    uboot.set_env("fdt_addr", "0x40000000").await.unwrap();
    info!("set fdt_addr ok");
    assert_eq!(uboot.env_int("fdt_addr").await.unwrap(), 0x40000000);
    let _ = out.kill();
    out.wait().unwrap();
}

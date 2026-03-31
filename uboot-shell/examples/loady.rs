use std::process::{Child, Command};

use log::{debug, info};
use tokio::{
    net::TcpStream,
    time::{Duration, sleep},
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use uboot_shell::UbootShell;

#[tokio::main]
async fn main() {
    env_logger::init();

    let (mut out, mut uboot) = new_uboot().await;

    uboot
        .loady(0x40200000, "Cargo.toml", |r, a| {
            debug!("{r}/{a}");
        })
        .await
        .unwrap();

    info!("finish");
    let _ = out.kill();
    let _ = out.wait();
}

async fn new_uboot() -> (Child, UbootShell) {
    // qemu-system-aarch64 -machine virt -cpu cortex-a57 -nographic -bios assets/u-boot.bin -serial tcp::12345,server
    let out = Command::new("qemu-system-aarch64")
        .args([
            "-machine",
            "virt",
            "-cpu",
            "cortex-a57",
            "-nographic",
            "-serial",
            "tcp::12345,server",
            "-bios",
            "assets/u-boot.bin",
        ])
        .spawn()
        .unwrap();

    loop {
        sleep(Duration::from_millis(100)).await;
        match TcpStream::connect("127.0.0.1:12345").await {
            Ok(s) => {
                let (rx, tx) = s.into_split();
                println!("connect ok");
                return (
                    out,
                    UbootShell::new(tx.compat_write(), rx.compat())
                        .await
                        .unwrap(),
                );
            }
            Err(e) => {
                println!("wait for qemu serial port ready: {e}");
            }
        }
    }
}

use std::fmt;

use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use tokio_serial::SerialPortBuilderExt;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use uboot_shell::UbootShell;

#[tokio::main]
async fn main() {
    println!("wait for uboot");

    // dd if=/dev/zero of=target/test.txt bs=1k count=258
    let file = "target/test.txt";

    let mut uboot = new_uboot().await;

    // let addr = uboot.env_int("kernel_addr_r").unwrap();
    let addr = 0x90000000;

    let pb = ProgressBar::new(100);
    pb.set_style(ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .unwrap()
        .with_key("eta", |state: &ProgressState, w: &mut dyn fmt::Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
        .progress_chars("#>-"));

    uboot
        .loady(addr, file, |r, a| {
            pb.set_length(a as _);
            pb.set_position(r as _);
        })
        .await
        .unwrap();
    pb.finish_with_message("upload done");
    println!("finish");

    uboot
        .loady(addr, file, |r, a| {
            pb.set_length(a as _);
            pb.set_position(r as _);
        })
        .await
        .unwrap();
    pb.finish_with_message("upload done");
    println!("finish2");
}

async fn new_uboot() -> UbootShell {
    let port = "/dev/ttyUSB0";
    let baud = 115200;
    // let baud = 1500000;

    let serial = tokio_serial::new(port, baud)
        .timeout(std::time::Duration::from_millis(3000))
        .open_native_async()
        .map_err(|e| format!("无法打开串口: {:?}", e))
        .unwrap();
    let (rx, tx) = tokio::io::split(serial);
    UbootShell::new(tx.compat_write(), rx.compat())
        .await
        .unwrap()
}

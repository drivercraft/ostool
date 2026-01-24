//! Serial terminal implementation.
//!
//! This module provides an interactive serial terminal for communication
//! with embedded devices and development boards. It supports:
//!
//! - Full keyboard input with special key sequences
//! - Line-based output callback for pattern matching
//! - Raw terminal mode for proper character handling
//!
//! # Exit Sequence
//!
//! Press `Ctrl+A` followed by `x` to exit the serial terminal.

use std::io::{self, Read, Write};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use futures::stream::StreamExt;
use tokio::task::{AbortHandle, spawn_blocking};

type Tx = Box<dyn Write + Send>;
type Rx = Box<dyn Read + Send>;
type OnlineCallback = Box<dyn Fn(&TermHandle, &str) + Send + Sync>;

/// Interactive serial terminal.
///
/// `SerialTerm` provides a bidirectional terminal interface over serial ports,
/// allowing users to interact with embedded devices in real-time.
///
/// # Example
///
/// ```rust,no_run
/// use ostool::sterm::SerialTerm;
///
/// // SerialTerm is typically used with serial port connections
/// // See run::uboot for usage examples
/// ```
pub struct SerialTerm {
    tx: Arc<Mutex<Tx>>,
    rx: Arc<Mutex<Rx>>,
    on_line: Option<OnlineCallback>,
}

/// Handle for controlling the terminal session.
///
/// Provides methods to stop the terminal from within callbacks.
pub struct TermHandle {
    is_running: AtomicBool,
}

impl TermHandle {
    /// Stops the terminal session.
    ///
    /// This can be called from within a line callback to terminate the session
    /// when a specific pattern is detected.
    pub fn stop(&self) {
        self.is_running
            .store(false, std::sync::atomic::Ordering::Release);
    }

    /// Returns whether the terminal session is still running.
    pub fn is_running(&self) -> bool {
        self.is_running.load(std::sync::atomic::Ordering::Acquire)
    }
}

// 特殊键序列状态
#[derive(Debug, PartialEq)]
enum KeySequenceState {
    Normal,
    CtrlAPressed,
}

impl SerialTerm {
    /// Creates a new serial terminal with the given read/write streams.
    ///
    /// # Arguments
    ///
    /// * `tx` - Writer for sending data to the serial port.
    /// * `rx` - Reader for receiving data from the serial port.
    /// * `on_line` - Callback function invoked for each complete line received.
    pub fn new<F>(tx: Tx, rx: Rx, on_line: F) -> Self
    where
        F: Fn(&TermHandle, &str) + Send + Sync + 'static,
    {
        SerialTerm {
            tx: Arc::new(Mutex::new(tx)),
            rx: Arc::new(Mutex::new(rx)),
            on_line: Some(Box::new(on_line)),
        }
    }

    /// Runs the interactive serial terminal.
    ///
    /// This method blocks until the user exits (Ctrl+A x) or the line callback
    /// calls `TermHandle::stop()`.
    ///
    /// # Errors
    ///
    /// Returns an error if terminal mode cannot be set or I/O fails.
    pub async fn run(&mut self) -> anyhow::Result<()> {
        // 启用raw模式

        // execute!(io::stdout(), Clear(ClearType::All))?;

        // 设置清理函数
        let cleanup_needed = enable_raw_mode().is_ok();

        let result = self.run_terminal().await;

        // 确保清理终端状态
        if cleanup_needed {
            let _ = disable_raw_mode();
            println!(); // 添加换行符
            eprintln!("✓ 已退出串口终端模式");
        }

        result
    }

    async fn run_terminal(&mut self) -> anyhow::Result<()> {
        let tx_port = self.tx.clone();
        let rx_port = self.rx.clone();

        let on_line = self.on_line.take().unwrap();

        let handle = Arc::new(TermHandle {
            is_running: AtomicBool::new(true),
        });

        // 使用 EventStream 异步处理键盘事件
        let tx_handle = tokio::spawn(Self::tx_work_async(handle.clone(), tx_port));

        let tx_abort = tx_handle.abort_handle();
        // 启动串口接收线程
        let rx_handle = spawn_blocking({
            let handle = handle.clone();
            move || Self::handle_serial_receive(rx_port, handle, tx_abort, on_line)
        });
        // 等待接收线程结束
        let _ = rx_handle.await?;
        let _ = tx_handle.await;
        info!("Serial terminal exited");
        Ok(())
    }

    fn handle_serial_receive<F>(
        rx_port: Arc<Mutex<Rx>>,
        handle: Arc<TermHandle>,
        tx_abort: AbortHandle,
        on_line: F,
    ) -> io::Result<()>
    where
        F: Fn(&TermHandle, &str) + Send + Sync + 'static,
    {
        let mut buffer = [0u8; 1024];
        let mut byte = [0u8; 1];
        let mut line = Vec::with_capacity(0x1000);

        while handle.is_running() {
            // 从串口读取数据
            match rx_port.lock().unwrap().read(&mut buffer) {
                Ok(bytes_read) if bytes_read > 0 => {
                    // 将数据直接写入stdout
                    let data = &buffer[..bytes_read];
                    for &b in data {
                        line.push(b);
                        if b == b'\n' {
                            byte[0] = b'\r';
                            io::stdout().write_all(&byte)?;
                            let line_str = String::from_utf8_lossy(&line);
                            (on_line)(handle.as_ref(), &line_str);
                            line.clear();
                        }
                        byte[0] = b;
                        io::stdout().write_all(&byte)?;
                    }

                    io::stdout().flush()?;
                }
                Ok(_) => {
                    // 没有数据可读，短暂休眠
                    thread::sleep(Duration::from_millis(1));
                }
                Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                    // 超时是正常的，继续
                    if handle.is_running() {
                        continue;
                    } else {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("\n串口读取错误: {}", e);
                    break;
                }
            }
        }
        tx_abort.abort();
        Ok(())
    }

    fn send_key_to_serial(
        tx_port: &Arc<Mutex<Tx>>,
        key: crossterm::event::KeyEvent,
    ) -> io::Result<()> {
        let mut bytes = Vec::new();

        // 处理字符键
        match key.code {
            KeyCode::Char(c) => {
                Self::handle_character_key(c, key.modifiers, &mut bytes);
            }
            // 基本控制键
            KeyCode::Enter => Self::handle_enter_key(key.modifiers, &mut bytes),
            KeyCode::Backspace => Self::handle_backspace_key(key.modifiers, &mut bytes),
            KeyCode::Tab => Self::handle_tab_key(key.modifiers, &mut bytes),
            KeyCode::Esc => {
                // Esc本身加上可能的修饰符
                if key.modifiers.contains(KeyModifiers::ALT) {
                    bytes.extend_from_slice(&[0x1b, 0x1b]); // Alt+Esc
                } else {
                    bytes.push(0x1b);
                }
            }
            // 光标控制键
            KeyCode::Up => Self::handle_arrow_key(key.code, key.modifiers, &mut bytes),
            KeyCode::Down => Self::handle_arrow_key(key.code, key.modifiers, &mut bytes),
            KeyCode::Left => Self::handle_arrow_key(key.code, key.modifiers, &mut bytes),
            KeyCode::Right => Self::handle_arrow_key(key.code, key.modifiers, &mut bytes),
            // 编辑键
            KeyCode::Home => Self::handle_home_end_key(key.code, key.modifiers, &mut bytes),
            KeyCode::End => Self::handle_home_end_key(key.code, key.modifiers, &mut bytes),
            KeyCode::PageUp => Self::handle_page_key(key.code, key.modifiers, &mut bytes),
            KeyCode::PageDown => Self::handle_page_key(key.code, key.modifiers, &mut bytes),
            KeyCode::Insert => Self::handle_insert_key(key.modifiers, &mut bytes),
            KeyCode::Delete => Self::handle_delete_key(key.modifiers, &mut bytes),
            // 功能键
            KeyCode::F(n) => Self::handle_function_key(n, key.modifiers, &mut bytes),
            // 其他特殊键
            KeyCode::Null => {}
            KeyCode::CapsLock => {}
            KeyCode::ScrollLock => {}
            KeyCode::NumLock => {}
            KeyCode::PrintScreen => {}
            KeyCode::Pause => {}
            KeyCode::Menu => {}
            KeyCode::KeypadBegin => {}
            KeyCode::Media(_) => {}
            KeyCode::Modifier(_) => {}
            _ => {}
        }

        if !bytes.is_empty() {
            tx_port.lock().unwrap().write_all(&bytes)?;
            tx_port.lock().unwrap().flush()?;
        }

        Ok(())
    }

    fn handle_character_key(c: char, modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
        if modifiers.contains(KeyModifiers::CONTROL) {
            // Ctrl 组合键
            let ctrl_char = match c {
                'a'..='z' => ((c as u8 - b'a') + 1) as char,
                'A'..='Z' => ((c as u8 - b'A') + 1) as char,
                '2' => '\x00', // Ctrl+2 (Null)
                '3' => '\x1b', // Ctrl+3 (Esc)
                '4' => '\x1c', // Ctrl+4 (File Separator)
                '5' => '\x1d', // Ctrl+5 (Group Separator)
                '6' => '\x1e', // Ctrl+6 (Record Separator)
                '7' => '\x1f', // Ctrl+7 (Unit Separator)
                '8' => '\x7f', // Ctrl+8 (Delete)
                '?' => '\x7f', // Ctrl+? (Delete)
                '[' => '\x1b', // Ctrl+[ (Esc)
                ']' => '\x1d', // Ctrl+] (Group Separator)
                '^' => '\x1e', // Ctrl+^ (Record Separator)
                '_' => '\x1f', // Ctrl+_ (Unit Separator)
                _ => c,
            };
            bytes.push(ctrl_char as u8);
        } else if modifiers.contains(KeyModifiers::ALT) {
            // Alt 组合键 - 发送ESC前缀
            bytes.push(0x1b);
            bytes.push(c as u8);
        } else {
            // 普通字符
            bytes.push(c as u8);
        }
    }

    fn handle_enter_key(modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
        if modifiers.contains(KeyModifiers::ALT) {
            bytes.extend_from_slice(&[0x1b, b'\r']); // Alt+Enter
        } else if modifiers.contains(KeyModifiers::SHIFT) {
            bytes.extend_from_slice(&[0x1b, b'[', b'Z']); // Shift+Enter (在某些终端中)
        } else {
            bytes.push(b'\r');
        }
    }

    fn handle_backspace_key(modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
        if modifiers.contains(KeyModifiers::ALT) {
            bytes.extend_from_slice(&[0x1b, 0x7f]); // Alt+Backspace
        } else if modifiers.contains(KeyModifiers::CONTROL) {
            bytes.push(b'\x08'); // Ctrl+Backspace (Ctrl+H)
        } else {
            bytes.push(0x7f); // 普通Backspace
        }
    }

    fn handle_tab_key(modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
        if modifiers.contains(KeyModifiers::SHIFT) {
            // Shift+Tab
            bytes.extend_from_slice(&[0x1b, b'[', b'Z']);
        } else if modifiers.contains(KeyModifiers::ALT) {
            bytes.extend_from_slice(&[0x1b, b'\t']); // Alt+Tab
        } else if modifiers.contains(KeyModifiers::CONTROL) {
            bytes.extend_from_slice(&[0x1b, b'[', b'I']); // Ctrl+Tab
        } else {
            bytes.push(b'\t');
        }
    }

    fn handle_arrow_key(key: KeyCode, modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
        let base_sequence = match key {
            KeyCode::Up => b'A',
            KeyCode::Down => b'B',
            KeyCode::Right => b'C',
            KeyCode::Left => b'D',
            _ => return,
        };

        if modifiers.contains(KeyModifiers::ALT) {
            // Alt + 箭头键 (应用模式)
            bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'3', base_sequence]);
        } else if modifiers.contains(KeyModifiers::SHIFT) {
            // Shift + 箭头键 (选择模式)
            bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'2', base_sequence]);
        } else if modifiers.contains(KeyModifiers::CONTROL) {
            // Ctrl + 箭头键 (单词跳跃)
            bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'5', base_sequence]);
        } else {
            // 普通箭头键
            bytes.extend_from_slice(&[0x1b, b'[', base_sequence]);
        }
    }

    fn handle_home_end_key(key: KeyCode, modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
        let base_sequence = match key {
            KeyCode::Home => b'H',
            KeyCode::End => b'F',
            _ => return,
        };

        if modifiers.contains(KeyModifiers::SHIFT) {
            // Shift + Home/End
            bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'2', base_sequence]);
        } else if modifiers.contains(KeyModifiers::CONTROL) {
            // Ctrl + Home/End
            bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'5', base_sequence]);
        } else {
            // 普通Home/End
            bytes.extend_from_slice(&[0x1b, b'[', base_sequence]);
        }
    }

    fn handle_page_key(key: KeyCode, modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
        let base_sequence = match key {
            KeyCode::PageUp => b'5',
            KeyCode::PageDown => b'6',
            _ => return,
        };

        if modifiers.contains(KeyModifiers::SHIFT) {
            // Shift + PageUp/Down
            bytes.extend_from_slice(&[0x1b, b'[', base_sequence, b';', b'2', b'~']);
        } else if modifiers.contains(KeyModifiers::CONTROL) {
            // Ctrl + PageUp/Down
            bytes.extend_from_slice(&[0x1b, b'[', base_sequence, b';', b'5', b'~']);
        } else if modifiers.contains(KeyModifiers::ALT) {
            // Alt + PageUp/Down
            bytes.extend_from_slice(&[0x1b, b'[', base_sequence, b';', b'3', b'~']);
        } else {
            // 普通PageUp/Down
            bytes.extend_from_slice(&[0x1b, b'[', base_sequence, b'~']);
        }
    }

    fn handle_insert_key(modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
        if modifiers.contains(KeyModifiers::SHIFT) {
            bytes.extend_from_slice(&[0x1b, b'[', b'2', b';', b'2', b'~']); // Shift+Insert
        } else if modifiers.contains(KeyModifiers::CONTROL) {
            bytes.extend_from_slice(&[0x1b, b'[', b'2', b';', b'5', b'~']); // Ctrl+Insert
        } else {
            bytes.extend_from_slice(&[0x1b, b'[', b'2', b'~']); // 普通Insert
        }
    }

    fn handle_delete_key(modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
        if modifiers.contains(KeyModifiers::SHIFT) {
            bytes.extend_from_slice(&[0x1b, b'[', b'3', b';', b'2', b'~']); // Shift+Delete
        } else if modifiers.contains(KeyModifiers::CONTROL) {
            bytes.extend_from_slice(&[0x1b, b'[', b'3', b';', b'5', b'~']); // Ctrl+Delete
        } else if modifiers.contains(KeyModifiers::ALT) {
            bytes.extend_from_slice(&[0x1b, b'[', b'3', b';', b'3', b'~']); // Alt+Delete
        } else {
            bytes.extend_from_slice(&[0x1b, b'[', b'3', b'~']); // 普通Delete
        }
    }

    fn handle_function_key(n: u8, modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
        match n {
            1..=4 => {
                // F1-F4 使用 SS3序列
                let f_char = match n {
                    1 => b'P',
                    2 => b'Q',
                    3 => b'R',
                    4 => b'S',
                    _ => return,
                };

                if modifiers.contains(KeyModifiers::SHIFT) {
                    bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'2', f_char]);
                } else if modifiers.contains(KeyModifiers::ALT) {
                    bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'3', f_char]);
                } else if modifiers.contains(KeyModifiers::CONTROL) {
                    bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'5', f_char]);
                } else {
                    bytes.extend_from_slice(&[0x1b, b'O', f_char]);
                }
            }
            5..=12 => {
                // F5-F12 使用 CSI序列
                let f_sequence = match n {
                    5 => &b"15"[..],
                    6 => &b"17"[..],
                    7 => &b"18"[..],
                    8 => &b"19"[..],
                    9 => &b"20"[..],
                    10 => &b"21"[..],
                    11 => &b"23"[..],
                    12 => &b"24"[..],
                    _ => return,
                };

                if modifiers.contains(KeyModifiers::SHIFT) {
                    bytes.extend_from_slice(&[0x1b, b'[']);
                    bytes.extend_from_slice(f_sequence);
                    bytes.extend_from_slice(b";2~");
                } else if modifiers.contains(KeyModifiers::ALT) {
                    bytes.extend_from_slice(&[0x1b, b'[']);
                    bytes.extend_from_slice(f_sequence);
                    bytes.extend_from_slice(b";3~");
                } else if modifiers.contains(KeyModifiers::CONTROL) {
                    bytes.extend_from_slice(&[0x1b, b'[']);
                    bytes.extend_from_slice(f_sequence);
                    bytes.extend_from_slice(b";5~");
                } else {
                    bytes.extend_from_slice(&[0x1b, b'[']);
                    bytes.extend_from_slice(f_sequence);
                    bytes.push(b'~');
                }
            }
            13..=24 => {
                // F13-F24 (扩展功能键)
                let f_num = n + 12; // F13 -> 25, F24 -> 36
                let f_str = f_num.to_string();

                if modifiers.contains(KeyModifiers::SHIFT) {
                    bytes.extend_from_slice(&[0x1b, b'[']);
                    bytes.extend_from_slice(f_str.as_bytes());
                    bytes.extend_from_slice(b";2~");
                } else if modifiers.contains(KeyModifiers::ALT) {
                    bytes.extend_from_slice(&[0x1b, b'[']);
                    bytes.extend_from_slice(f_str.as_bytes());
                    bytes.extend_from_slice(b";3~");
                } else if modifiers.contains(KeyModifiers::CONTROL) {
                    bytes.extend_from_slice(&[0x1b, b'[']);
                    bytes.extend_from_slice(f_str.as_bytes());
                    bytes.extend_from_slice(b";5~");
                } else {
                    bytes.extend_from_slice(&[0x1b, b'[']);
                    bytes.extend_from_slice(f_str.as_bytes());
                    bytes.push(b'~');
                }
            }
            _ => {}
        }
    }

    fn send_ctrl_a_to_serial(tx_port: &Arc<Mutex<Tx>>) -> io::Result<()> {
        tx_port.lock().unwrap().write_all(&[0x01])?; // Ctrl+A
        tx_port.lock().unwrap().flush()?;
        Ok(())
    }

    async fn tx_work_async(handle: Arc<TermHandle>, tx_port: Arc<Mutex<Tx>>) -> anyhow::Result<()> {
        // 使用 EventStream 异步处理键盘事件
        let mut reader = EventStream::new();
        let mut key_state = KeySequenceState::Normal;

        while handle.is_running() {
            // 使用 EventStream::next() 异步等待事件，不会阻塞
            match reader.next().await {
                Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                    // 检测 Ctrl+A+x 退出序列
                    match key_state {
                        KeySequenceState::Normal => {
                            if key.code == KeyCode::Char('a')
                                && key.modifiers.contains(KeyModifiers::CONTROL)
                            {
                                key_state = KeySequenceState::CtrlAPressed;
                            } else {
                                // 普通按键，发送到串口
                                if let Err(e) = Self::send_key_to_serial(&tx_port, key) {
                                    eprintln!("\r\n发送按键失败: {}", e);
                                }
                            }
                        }
                        KeySequenceState::CtrlAPressed => {
                            if key.code == KeyCode::Char('x') {
                                // 用户请求退出
                                eprintln!("\r\nExit by: Ctrl+A+x");
                                handle.stop();
                                break;
                            } else {
                                // 不是x键，发送上一个按键并重置状态
                                if key.code != KeyCode::Char('a') {
                                    if let Err(e) = Self::send_ctrl_a_to_serial(&tx_port) {
                                        eprintln!("\r\n发送 Ctrl+A 失败: {}", e);
                                    }
                                    if let Err(e) = Self::send_key_to_serial(&tx_port, key) {
                                        eprintln!("\r\n发送按键失败: {}", e);
                                    }
                                    key_state = KeySequenceState::Normal;
                                }
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    eprintln!("\r\n键盘事件错误: {}", e);
                    break;
                }
                None => {
                    // EventStream 结束
                    break;
                }
                Some(Ok(_)) => {
                    // 忽略非按键事件（鼠标、调整大小等）
                }
            }
        }

        Ok(())
    }
}

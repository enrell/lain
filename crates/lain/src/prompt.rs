//! A password prompt without echo, with no crate: termios through
//! `libc` on Unix, the console mode through two kernel32 calls on
//! Windows. When stdin is not a terminal the line is read as is.

use std::io::Write;

fn read_line() -> Result<String, String> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| e.to_string())?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

pub fn password(prompt: &str) -> Result<String, String> {
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let restore = echo_off();
    let line = read_line();
    if let Some(restore) = restore {
        restore();
        eprintln!();
    }
    line
}

#[cfg(unix)]
fn echo_off() -> Option<Box<dyn FnOnce()>> {
    // SAFETY: tcgetattr/tcsetattr on fd 0 with a zeroed, fully written
    // termios; on failure (not a terminal) nothing is changed.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut t) != 0 {
            return None;
        }
        let saved = t;
        t.c_lflag &= !libc::ECHO;
        if libc::tcsetattr(0, libc::TCSANOW, &t) != 0 {
            return None;
        }
        Some(Box::new(move || {
            libc::tcsetattr(0, libc::TCSANOW, &saved);
        }))
    }
}

#[cfg(windows)]
fn echo_off() -> Option<Box<dyn FnOnce()>> {
    type Handle = *mut core::ffi::c_void;
    const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    const ENABLE_ECHO_INPUT: u32 = 0x0004;
    unsafe extern "system" {
        fn GetStdHandle(n: u32) -> Handle;
        fn GetConsoleMode(h: Handle, mode: *mut u32) -> i32;
        fn SetConsoleMode(h: Handle, mode: u32) -> i32;
    }
    // SAFETY: documented kernel32 calls on the process's own stdin
    // handle; a handle that is not a console fails GetConsoleMode and
    // nothing is changed.
    unsafe {
        let h = GetStdHandle(STD_INPUT_HANDLE);
        let mut mode = 0u32;
        if h.is_null() || GetConsoleMode(h, &mut mode) == 0 {
            return None;
        }
        if SetConsoleMode(h, mode & !ENABLE_ECHO_INPUT) == 0 {
            return None;
        }
        let h = h as usize;
        Some(Box::new(move || {
            SetConsoleMode(h as Handle, mode);
        }))
    }
}

#[cfg(not(any(unix, windows)))]
fn echo_off() -> Option<Box<dyn FnOnce()>> {
    None
}

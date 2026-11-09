//! Platform glue: the native window handle, exact window placement on a
//! given monitor without activation (test runs), stderr to a file, and a
//! capture of this app's own window for screenshots. Windows has the
//! real implementations; elsewhere they are no-ops or simple fallbacks.

use gpui::Window;

use crate::Args;
use crate::app::LainApp;

pub fn window_handle(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        #[cfg(windows)]
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        RawWindowHandle::Xlib(h) => Some(h.window as isize),
        RawWindowHandle::Xcb(h) => Some(h.window.get() as isize),
        RawWindowHandle::Wayland(_) => Some(1),
        _ => None,
    }
}

/// `--log <file>`: the release exe has no console, so stderr goes to a file.
#[cfg(windows)]
pub fn redirect_stderr(path: &str) {
    use std::os::windows::io::IntoRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{STD_ERROR_HANDLE, SetStdHandle};
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let h = HANDLE(f.into_raw_handle());
        unsafe {
            let _ = SetStdHandle(STD_ERROR_HANDLE, h);
        }
    }
}

#[cfg(unix)]
pub fn redirect_stderr(path: &str) {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn dup2(old: i32, new: i32) -> i32;
    }
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        unsafe {
            dup2(f.as_raw_fd(), 2);
        }
        std::mem::forget(f);
    }
}

/// Moves the window while it is still hidden (Windows test placement).
pub fn place_early(window: &Window, args: &Args) {
    #[cfg(windows)]
    if let (Some((x, y)), Some(h)) = (args.window_pos, window_handle(window)) {
        let (w, hh) = args.window_size.unwrap_or((1440, 860));
        place_window(h, x, y, w, hh, false);
    }
    #[cfg(not(windows))]
    let _ = (window, args);
}

/// Shows the placed window once GPUI's setup and the DPI change settled.
pub fn tick_placement(app: &mut LainApp, n: u64) {
    #[cfg(windows)]
    if n == 3
        && let Some((x, y)) = app.args.window_pos
    {
        let (w, h) = app.args.window_size.unwrap_or((1440, 860));
        place_window(app.hwnd, x, y, w, h, true);
    }
    #[cfg(not(windows))]
    let _ = (app, n);
}

/// GPUI 0.2.2 does not honor a requested origin on a secondary monitor
/// with mixed DPI, so the window is created hidden, moved with
/// `SetWindowPos` and shown with `SW_SHOWNA` (no activation; see the
/// README's "Window placement").
#[cfg(windows)]
fn place_window(hwnd: isize, x: i32, y: i32, w: i32, h: i32, show: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SW_SHOWNA, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, ShowWindow};
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    unsafe {
        if let Err(e) = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER) {
            eprintln!("window placement: {e}");
        }
        if show {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
            let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
        }
    }
}

/// Captures this app's own window (PrintWindow with full content, which
/// includes mpv's video surface) into a PNG.
#[cfg(windows)]
pub fn capture(hwnd: isize, out: &std::path::Path) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
    };
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    unsafe {
        let mut rc = RECT::default();
        GetClientRect(hwnd, &mut rc).map_err(|e| e.to_string())?;
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        if w <= 0 || h <= 0 {
            return Err("empty window".into());
        }
        let wdc = GetDC(Some(hwnd));
        let mdc = CreateCompatibleDC(Some(wdc));
        let bmp = CreateCompatibleBitmap(wdc, w, h);
        let old = SelectObject(mdc, bmp.into());
        // PW_CLIENTONLY | PW_RENDERFULLCONTENT
        let ok = PrintWindow(hwnd, mdc, PRINT_WINDOW_FLAGS(1 | 2)).as_bool();
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(mdc, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        SelectObject(mdc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mdc);
        ReleaseDC(Some(hwnd), wdc);
        if !ok || lines == 0 {
            return Err("PrintWindow failed".into());
        }
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        let img = image::RgbaImage::from_raw(w as u32, h as u32, buf).ok_or("bad bitmap")?;
        if let Some(dir) = out.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        img.save(out).map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
pub fn capture(_hwnd: isize, _out: &std::path::Path) -> Result<(), String> {
    Err("window capture is only implemented on Windows".into())
}

/// Opens a folder in the system file manager.
pub fn open_folder(path: &std::path::Path) {
    let prog = if cfg!(windows) { "explorer" } else { "xdg-open" };
    if let Err(e) = std::process::Command::new(prog).arg(path).spawn() {
        eprintln!("open {}: {e}", path.display());
    }
}

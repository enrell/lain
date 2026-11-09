//! Minimal hand-written libmpv FFI: the client API (commands, properties,
//! events) and the render API with the OpenGL backend. Only what the
//! prototype uses; linked against `libmpv-2.dll` through `mpv.lib`.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;

#[repr(C)]
pub struct MpvHandle {
    _p: [u8; 0],
}
#[repr(C)]
pub struct MpvRenderContext {
    _p: [u8; 0],
}

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct OpenGlInitParams {
    get_proc_address: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void>,
    get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
struct OpenGlFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

#[repr(C)]
struct RawEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct RawEventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct RawEndFile {
    reason: c_int,
    error: c_int,
    playlist_entry_id: i64,
    playlist_insert_id: i64,
    playlist_insert_num_entries: c_int,
}

const FORMAT_NONE: c_int = 0;
const FORMAT_STRING: c_int = 1;
const FORMAT_FLAG: c_int = 3;
const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;

const RENDER_PARAM_INVALID: c_int = 0;
const RENDER_PARAM_API_TYPE: c_int = 1;
const RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
const RENDER_PARAM_OPENGL_FBO: c_int = 3;
const RENDER_PARAM_FLIP_Y: c_int = 4;
const RENDER_PARAM_ADVANCED_CONTROL: c_int = 10;
const RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;

pub const RENDER_UPDATE_FRAME: u64 = 1;

#[link(name = "mpv")]
unsafe extern "C" {
    fn mpv_create() -> *mut MpvHandle;
    fn mpv_initialize(ctx: *mut MpvHandle) -> c_int;
    fn mpv_terminate_destroy(ctx: *mut MpvHandle);
    fn mpv_set_option_string(ctx: *mut MpvHandle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_command(ctx: *mut MpvHandle, args: *mut *const c_char) -> c_int;
    fn mpv_set_property_string(ctx: *mut MpvHandle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_get_property(ctx: *mut MpvHandle, name: *const c_char, format: c_int, data: *mut c_void) -> c_int;
    fn mpv_get_property_string(ctx: *mut MpvHandle, name: *const c_char) -> *mut c_char;
    fn mpv_observe_property(ctx: *mut MpvHandle, userdata: u64, name: *const c_char, format: c_int) -> c_int;
    fn mpv_wait_event(ctx: *mut MpvHandle, timeout: f64) -> *mut RawEvent;
    fn mpv_wakeup(ctx: *mut MpvHandle);
    fn mpv_free(data: *mut c_void);
    fn mpv_error_string(error: c_int) -> *const c_char;

    fn mpv_render_context_create(res: *mut *mut MpvRenderContext, mpv: *mut MpvHandle, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_set_update_callback(
        ctx: *mut MpvRenderContext,
        callback: Option<unsafe extern "C" fn(*mut c_void)>,
        callback_ctx: *mut c_void,
    );
    fn mpv_render_context_update(ctx: *mut MpvRenderContext) -> u64;
    fn mpv_render_context_render(ctx: *mut MpvRenderContext, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_report_swap(ctx: *mut MpvRenderContext);
    fn mpv_render_context_free(ctx: *mut MpvRenderContext);
}

fn err(code: c_int) -> String {
    unsafe { CStr::from_ptr(mpv_error_string(code)).to_string_lossy().into_owned() }
}

fn cs(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

/// One mpv core. The client API is thread-safe, so this is shared freely.
pub struct Mpv {
    h: *mut MpvHandle,
}

unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

#[derive(Debug, Clone)]
pub enum Prop {
    None,
    Str(String),
    Flag(bool),
    Int(i64),
    Double(f64),
}

#[derive(Debug, Clone)]
pub enum Event {
    None,
    Shutdown,
    FileLoaded,
    EndFile { reason: i32, error: i32 },
    PlaybackRestart,
    PropertyChange { name: String, value: Prop },
    Other(i32),
}

#[derive(Clone, Copy)]
pub enum Format {
    Str,
    Flag,
    Int,
    Double,
}

impl Mpv {
    /// Creates a core with the given options set before initialization.
    pub fn new(options: &[(&str, &str)]) -> Result<Mpv, String> {
        let h = unsafe { mpv_create() };
        if h.is_null() {
            return Err("mpv_create failed".into());
        }
        let m = Mpv { h };
        for (k, v) in options {
            let r = unsafe { mpv_set_option_string(h, cs(k).as_ptr(), cs(v).as_ptr()) };
            if r < 0 {
                eprintln!("mpv option {k}={v}: {}", err(r));
            }
        }
        let r = unsafe { mpv_initialize(h) };
        if r < 0 {
            return Err(format!("mpv_initialize: {}", err(r)));
        }
        Ok(m)
    }

    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let owned: Vec<CString> = args.iter().map(|a| cs(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(ptr::null());
        let r = unsafe { mpv_command(self.h, ptrs.as_mut_ptr()) };
        if r < 0 { Err(format!("{}: {}", args.join(" "), err(r))) } else { Ok(()) }
    }

    pub fn set(&self, name: &str, value: &str) {
        let r = unsafe { mpv_set_property_string(self.h, cs(name).as_ptr(), cs(value).as_ptr()) };
        if r < 0 {
            eprintln!("mpv set {name}={value}: {}", err(r));
        }
    }

    pub fn get_str(&self, name: &str) -> Option<String> {
        let p = unsafe { mpv_get_property_string(self.h, cs(name).as_ptr()) };
        if p.is_null() {
            return None;
        }
        let s = unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() };
        unsafe { mpv_free(p as *mut c_void) };
        Some(s)
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        let mut v = 0f64;
        let r = unsafe { mpv_get_property(self.h, cs(name).as_ptr(), FORMAT_DOUBLE, &mut v as *mut f64 as *mut c_void) };
        (r >= 0).then_some(v)
    }

    pub fn get_i64(&self, name: &str) -> Option<i64> {
        let mut v = 0i64;
        let r = unsafe { mpv_get_property(self.h, cs(name).as_ptr(), FORMAT_INT64, &mut v as *mut i64 as *mut c_void) };
        (r >= 0).then_some(v)
    }

    pub fn observe(&self, name: &str, format: Format) {
        let f = match format {
            Format::Str => FORMAT_STRING,
            Format::Flag => FORMAT_FLAG,
            Format::Int => FORMAT_INT64,
            Format::Double => FORMAT_DOUBLE,
        };
        unsafe { mpv_observe_property(self.h, 0, cs(name).as_ptr(), f) };
    }

    pub fn wakeup(&self) {
        unsafe { mpv_wakeup(self.h) }
    }

    /// Blocks up to `timeout` seconds. Call from one thread only.
    pub fn wait_event(&self, timeout: f64) -> Event {
        let e = unsafe { &*mpv_wait_event(self.h, timeout) };
        match e.event_id {
            0 => Event::None,
            1 => Event::Shutdown,
            7 => {
                let d = unsafe { &*(e.data as *const RawEndFile) };
                Event::EndFile { reason: d.reason, error: d.error }
            }
            8 => Event::FileLoaded,
            21 => Event::PlaybackRestart,
            22 => {
                let p = unsafe { &*(e.data as *const RawEventProperty) };
                let name = unsafe { CStr::from_ptr(p.name).to_string_lossy().into_owned() };
                let value = unsafe {
                    match p.format {
                        FORMAT_NONE => Prop::None,
                        FORMAT_STRING => {
                            let s = *(p.data as *const *const c_char);
                            Prop::Str(CStr::from_ptr(s).to_string_lossy().into_owned())
                        }
                        FORMAT_FLAG => Prop::Flag(*(p.data as *const c_int) != 0),
                        FORMAT_INT64 => Prop::Int(*(p.data as *const i64)),
                        FORMAT_DOUBLE => Prop::Double(*(p.data as *const f64)),
                        _ => Prop::None,
                    }
                };
                Event::PropertyChange { name, value }
            }
            id => Event::Other(id),
        }
    }

    pub fn raw(&self) -> *mut MpvHandle {
        self.h
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        unsafe { mpv_terminate_destroy(self.h) }
    }
}

// ---- OpenGL proc address (WGL, Windows) ---------------------------------

#[cfg(windows)]
mod gl {
    use std::ffi::{c_char, c_void};
    #[link(name = "opengl32")]
    unsafe extern "system" {
        fn wglGetProcAddress(name: *const c_char) -> *mut c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    }
    /// wglGetProcAddress only knows extensions and GL > 1.1; the 1.1 core
    /// lives in opengl32.dll itself (same as glutin does it).
    pub unsafe extern "C" fn get_proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
        unsafe {
            let p = wglGetProcAddress(name);
            let bad = p.is_null() || matches!(p as isize, 1 | 2 | 3 | -1);
            if !bad {
                return p;
            }
            let m = GetModuleHandleA(c"opengl32.dll".as_ptr());
            if m.is_null() { std::ptr::null_mut() } else { GetProcAddress(m, name) }
        }
    }
}

/// `glGetString(GL_RENDERER)` / `GL_VERSION` of the current context.
pub fn gl_info() -> String {
    unsafe {
        let p = gl::get_proc_address(ptr::null_mut(), c"glGetString".as_ptr());
        if p.is_null() {
            return "unknown".into();
        }
        let f: unsafe extern "system" fn(u32) -> *const c_char = std::mem::transmute(p);
        let get = |n: u32| {
            let s = f(n);
            if s.is_null() { String::new() } else { CStr::from_ptr(s).to_string_lossy().into_owned() }
        };
        format!("{} / {}", get(0x1F01), get(0x1F02))
    }
}

/// Puts the GL state mpv expects back: femtovg can leave GL_SCISSOR_TEST
/// enabled (mpv documents that it assumes the default, disabled), which
/// clips mpv's draw and leaves stale pixels in the letterbox.
pub fn gl_reset_for_mpv() {
    type Disable = unsafe extern "system" fn(u32);
    static DISABLE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let p = *DISABLE.get_or_init(|| unsafe { gl::get_proc_address(ptr::null_mut(), c"glDisable".as_ptr()) as usize });
    if p != 0 {
        let f: Disable = unsafe { std::mem::transmute(p) };
        unsafe { f(0x0C11) }; // GL_SCISSOR_TEST
    }
}

/// The render context: UI (GL) thread only.
pub struct RenderCtx {
    ctx: *mut MpvRenderContext,
    _cb: Option<Box<Box<dyn Fn() + Send + Sync>>>,
    // keeps the core alive until the render context is gone
    _mpv: std::sync::Arc<Mpv>,
}

unsafe extern "C" fn update_trampoline(data: *mut c_void) {
    let f = unsafe { &*(data as *const Box<dyn Fn() + Send + Sync>) };
    f();
}

impl RenderCtx {
    /// Must be called with the window's GL context current.
    pub fn new_gl(mpv: std::sync::Arc<Mpv>) -> Result<RenderCtx, String> {
        let api = c"opengl";
        let mut init = OpenGlInitParams { get_proc_address: Some(gl::get_proc_address), get_proc_address_ctx: ptr::null_mut() };
        let mut advanced: c_int = 1;
        let mut params = [
            RenderParam { kind: RENDER_PARAM_API_TYPE, data: api.as_ptr() as *mut c_void },
            RenderParam { kind: RENDER_PARAM_OPENGL_INIT_PARAMS, data: &mut init as *mut _ as *mut c_void },
            RenderParam { kind: RENDER_PARAM_ADVANCED_CONTROL, data: &mut advanced as *mut _ as *mut c_void },
            RenderParam { kind: RENDER_PARAM_INVALID, data: ptr::null_mut() },
        ];
        let mut ctx = ptr::null_mut();
        let r = unsafe { mpv_render_context_create(&mut ctx, mpv.raw(), params.as_mut_ptr()) };
        if r < 0 {
            return Err(format!("mpv_render_context_create: {}", err(r)));
        }
        Ok(RenderCtx { ctx, _cb: None, _mpv: mpv })
    }

    /// `f` runs on an mpv thread; it must only schedule a redraw.
    pub fn set_update_callback(&mut self, f: Box<dyn Fn() + Send + Sync>) {
        let b = Box::new(f);
        let p = &*b as *const Box<dyn Fn() + Send + Sync> as *mut c_void;
        unsafe { mpv_render_context_set_update_callback(self.ctx, Some(update_trampoline), p) };
        self._cb = Some(b);
    }

    pub fn update(&self) -> u64 {
        unsafe { mpv_render_context_update(self.ctx) }
    }

    /// Draws the current frame into `fbo` (0 = the window's back buffer).
    pub fn render(&self, fbo: i32, w: i32, h: i32) {
        let mut f = OpenGlFbo { fbo, w, h, internal_format: 0 };
        let mut flip: c_int = 1;
        let mut block: c_int = 0;
        let mut params = [
            RenderParam { kind: RENDER_PARAM_OPENGL_FBO, data: &mut f as *mut _ as *mut c_void },
            RenderParam { kind: RENDER_PARAM_FLIP_Y, data: &mut flip as *mut _ as *mut c_void },
            RenderParam { kind: RENDER_PARAM_BLOCK_FOR_TARGET_TIME, data: &mut block as *mut _ as *mut c_void },
            RenderParam { kind: RENDER_PARAM_INVALID, data: ptr::null_mut() },
        ];
        unsafe { mpv_render_context_render(self.ctx, params.as_mut_ptr()) };
    }

    pub fn report_swap(&self) {
        unsafe { mpv_render_context_report_swap(self.ctx) }
    }
}

impl Drop for RenderCtx {
    fn drop(&mut self) {
        unsafe {
            mpv_render_context_set_update_callback(self.ctx, None, ptr::null_mut());
            mpv_render_context_free(self.ctx);
        }
    }
}

//! External, optional crosshair-marker overlay for windowed-fullscreen Evolve.
//! This is a separate Windows process, not a graphics hook or game injection.
//! Arguments: Evolve PID, manager PID. Exits automatically if either quits.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[allow(unsafe_code)] // Isolated, minimal Win32/GDI interface for an external overlay window.
mod windows {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use std::sync::atomic::{AtomicU32, Ordering};

    type Handle = *mut c_void;
    type Hwnd = Handle;
    type Hdc = Handle;
    type Hbrush = Handle;
    type Hinstance = Handle;

    const WS_POPUP: u32 = 0x8000_0000;
    const WS_EX_TOPMOST: u32 = 0x0000_0008;
    const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
    const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    const WS_EX_LAYERED: u32 = 0x0008_0000;
    const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
    const LWA_COLORKEY: u32 = 1;
    const SW_HIDE: i32 = 0;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const WM_DESTROY: u32 = 2;
    const WM_PAINT: u32 = 15;
    const WM_TIMER: u32 = 0x0113;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_TIMEOUT: u32 = 0x0000_0102;
    const WIDTH: i32 = 190;
    const HEIGHT: i32 = 82;

    static GAME_PID: AtomicU32 = AtomicU32::new(0);
    static MANAGER_PID: AtomicU32 = AtomicU32::new(0);

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Message {
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: Point,
        lprivate: u32,
    }
    #[repr(C)]
    struct WindowClass {
        size: u32,
        style: u32,
        wnd_proc: Option<unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize>,
        cls_extra: i32,
        wnd_extra: i32,
        instance: Hinstance,
        icon: Handle,
        cursor: Handle,
        brush: Hbrush,
        menu_name: *const u16,
        class_name: *const u16,
        small_icon: Handle,
    }
    #[repr(C)]
    #[derive(Default)]
    struct PaintStruct {
        hdc: Hdc,
        erase: i32,
        paint: Rect,
        restore: i32,
        update: i32,
        reserved: [u8; 32],
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterClassExW(wc: *const WindowClass) -> u16;
        fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: Hwnd,
            menu: Handle,
            inst: Hinstance,
            args: *const c_void,
        ) -> Hwnd;
        fn DefWindowProcW(hwnd: Hwnd, msg: u32, wp: usize, lp: isize) -> isize;
        fn GetMessageW(msg: *mut Message, hwnd: Hwnd, low: u32, high: u32) -> i32;
        fn TranslateMessage(msg: *const Message) -> i32;
        fn DispatchMessageW(msg: *const Message) -> isize;
        fn DestroyWindow(hwnd: Hwnd) -> i32;
        fn PostQuitMessage(code: i32);
        fn SetLayeredWindowAttributes(hwnd: Hwnd, color: u32, alpha: u8, flags: u32) -> i32;
        fn SetTimer(hwnd: Hwnd, id: usize, delay: u32, fnptr: *const c_void) -> usize;
        fn SetWindowPos(hwnd: Hwnd, after: Hwnd, x: i32, y: i32, w: i32, h: i32, flags: u32)
        -> i32;
        fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
        fn GetForegroundWindow() -> Hwnd;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn GetClientRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
        fn ClientToScreen(hwnd: Hwnd, point: *mut Point) -> i32;
        fn BeginPaint(hwnd: Hwnd, paint: *mut PaintStruct) -> Hdc;
        fn EndPaint(hwnd: Hwnd, paint: *const PaintStruct) -> i32;
        fn FillRect(hdc: Hdc, rect: *const Rect, brush: Hbrush) -> i32;
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn GetStockObject(id: i32) -> Handle;
        fn CreatePen(style: i32, width: i32, color: u32) -> Handle;
        fn SelectObject(hdc: Hdc, object: Handle) -> Handle;
        fn DeleteObject(object: Handle) -> i32;
        fn Ellipse(hdc: Hdc, left: i32, top: i32, right: i32, bottom: i32) -> i32;
        fn MoveToEx(hdc: Hdc, x: i32, y: i32, old: *mut Point) -> i32;
        fn LineTo(hdc: Hdc, x: i32, y: i32) -> i32;
        fn SetBkMode(hdc: Hdc, mode: i32) -> i32;
        fn SetTextColor(hdc: Hdc, color: u32) -> u32;
        fn TextOutW(hdc: Hdc, x: i32, y: i32, text: *const u16, len: i32) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Hinstance;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    fn process_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        unsafe {
            let handle = OpenProcess(SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                return false;
            }
            let alive = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
            CloseHandle(handle);
            alive
        }
    }

    unsafe extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, wp: usize, lp: isize) -> isize {
        match msg {
            WM_TIMER => {
                if !process_alive(GAME_PID.load(Ordering::Relaxed))
                    || !process_alive(MANAGER_PID.load(Ordering::Relaxed))
                {
                    unsafe { DestroyWindow(hwnd) };
                    return 0;
                }
                let active = unsafe { GetForegroundWindow() };
                let mut pid = 0;
                unsafe { GetWindowThreadProcessId(active, &mut pid) };
                if pid != GAME_PID.load(Ordering::Relaxed) {
                    unsafe { ShowWindow(hwnd, SW_HIDE) };
                    return 0;
                }
                let mut rect = Rect::default();
                let mut point = Point::default();
                if unsafe { GetClientRect(active, &mut rect) } == 0
                    || unsafe { ClientToScreen(active, &mut point) } == 0
                {
                    return 0;
                }
                let cx = point.x + (rect.right - rect.left) / 2;
                let cy = point.y + (rect.bottom - rect.top) / 2;
                unsafe {
                    SetWindowPos(
                        hwnd,
                        (-1isize) as Hwnd,
                        cx - WIDTH / 2,
                        cy - 26,
                        WIDTH,
                        HEIGHT,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    )
                };
                0
            }
            WM_PAINT => {
                let mut ps = PaintStruct::default();
                let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
                if !hdc.is_null() {
                    let rect = Rect {
                        left: 0,
                        top: 0,
                        right: WIDTH,
                        bottom: HEIGHT,
                    };
                    unsafe { FillRect(hdc, &rect, GetStockObject(4)) };
                    let pen = unsafe { CreatePen(0, 2, 0x0050_50FF) };
                    if !pen.is_null() {
                        let old_pen = unsafe { SelectObject(hdc, pen) };
                        let old_brush = unsafe { SelectObject(hdc, GetStockObject(5)) };
                        unsafe {
                            Ellipse(hdc, WIDTH / 2 - 12, 14, WIDTH / 2 + 12, 38);
                            MoveToEx(hdc, WIDTH / 2 - 20, 26, null_mut());
                            LineTo(hdc, WIDTH / 2 - 15, 26);
                            MoveToEx(hdc, WIDTH / 2 + 15, 26, null_mut());
                            LineTo(hdc, WIDTH / 2 + 20, 26);
                            SelectObject(hdc, old_brush);
                            SelectObject(hdc, old_pen);
                            DeleteObject(pen);
                        }
                    }
                    let label: Vec<u16> = "ACTOR AIM · APPROX".encode_utf16().collect();
                    unsafe {
                        SetBkMode(hdc, 1);
                        SetTextColor(hdc, 0x00FF_FFFF);
                        TextOutW(hdc, 23, 49, label.as_ptr(), label.len() as i32);
                    }
                }
                unsafe { EndPaint(hwnd, &ps) };
                0
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
        }
    }

    pub fn run() {
        let mut args = std::env::args().skip(1);
        let (Some(game), Some(manager)) = (args.next(), args.next()) else {
            return;
        };
        let (Ok(game), Ok(manager)) = (game.parse::<u32>(), manager.parse::<u32>()) else {
            return;
        };
        if game == 0 || manager == 0 || !process_alive(game) || !process_alive(manager) {
            return;
        }
        GAME_PID.store(game, Ordering::Relaxed);
        MANAGER_PID.store(manager, Ordering::Relaxed);
        let class: Vec<u16> = "EvolveExternalAimPreview\0".encode_utf16().collect();
        unsafe {
            let instance = GetModuleHandleW(null());
            let wc = WindowClass {
                size: std::mem::size_of::<WindowClass>() as u32,
                style: 0,
                wnd_proc: Some(wnd_proc),
                cls_extra: 0,
                wnd_extra: 0,
                instance,
                icon: null_mut(),
                cursor: null_mut(),
                brush: null_mut(),
                menu_name: null(),
                class_name: class.as_ptr(),
                small_icon: null_mut(),
            };
            if RegisterClassExW(&wc) == 0 {
                return;
            }
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                WIDTH,
                HEIGHT,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                return;
            }
            SetLayeredWindowAttributes(hwnd, 0, 255, LWA_COLORKEY);
            SetTimer(hwnd, 1, 120, null());
            let mut msg = Message::default();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

#[cfg(windows)]
fn main() {
    windows::run()
}

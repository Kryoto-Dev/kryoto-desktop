//! A picture of a running game's window, for a community update's playtest
//! (community.rs): Kryoto Desktop takes it by itself while the player plays,
//! so the screenshot is of the build being offered and not a file from
//! anywhere. A copy of Kryoto Forge's launch-test capture
//! (kryoto-pipeline/src/launchtest.rs `windows_impl`); fix both together.
//!
//! Windows only. Elsewhere [`capture_png`] answers `None` and the player picks
//! a screenshot they took themselves.

/// The game's window (the process `pid` or anything it started) as a PNG.
/// `None` when it has no window yet, is minimised, covered, or drew black.
#[cfg(windows)]
pub fn capture_png(pid: u32) -> Option<Vec<u8>> {
    let hwnd = windows_impl::look(pid)?.hwnd?;
    let (w, h, mut bgra) = windows_impl::capture(hwnd)?;
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
    encode(w, h, &bgra)
}

#[cfg(not(windows))]
pub fn capture_png(_pid: u32) -> Option<Vec<u8>> {
    None
}

/// One window by its handle, as a PNG: Kryoto Desktop's own, for a report
/// from Kryoto Desktop Preview. A dark app is not a black game frame, so
/// nothing is thrown away for being dark. `None` when minimised.
#[cfg(windows)]
pub fn capture_window_png(hwnd: usize) -> Option<Vec<u8>> {
    let (w, h, mut bgra) = windows_impl::print(hwnd)?;
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
    encode(w, h, &bgra)
}

#[cfg(not(windows))]
pub fn capture_window_png(_hwnd: usize) -> Option<Vec<u8>> {
    None
}

/// RGBA, top row first, as a PNG.
pub fn encode(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(out)
}

#[cfg(windows)]
// Forge reads the window title and dialogs too; only the capture is used here.
#[allow(dead_code)]
mod windows_impl {
    //! The Windows half: the process table, and what is on screen.
    //!
    //! Both are read by calling into the OS directly rather than by adding a
    //! dependency, because both are one function each and the crates that wrap
    //! them bring a great deal that is not wanted in a build tool.

    #[derive(Clone)]
    pub struct Proc {
        pub pid: u32,
        pub parent: u32,
        pub name: String,
    }

    // The names are Win32's own. Renaming them to please a lint would mean the
    // signatures below no longer match the documentation anybody checks them
    // against, which is the only thing keeping an FFI declaration honest.
    #[allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]
    mod ffi {
        pub type HANDLE = *mut core::ffi::c_void;
        pub type HWND = *mut core::ffi::c_void;
        pub type BOOL = i32;
        pub type DWORD = u32;
        pub type LPARAM = isize;

        pub const TH32CS_SNAPPROCESS: DWORD = 0x0000_0002;
        pub const MAX_PATH: usize = 260;

        #[repr(C)]
        pub struct PROCESSENTRY32W {
            pub dwSize: DWORD,
            pub cntUsage: DWORD,
            pub th32ProcessID: DWORD,
            pub th32DefaultHeapID: usize,
            pub th32ModuleID: DWORD,
            pub cntThreads: DWORD,
            pub th32ParentProcessID: DWORD,
            pub pcPriClassBase: i32,
            pub dwFlags: DWORD,
            pub szExeFile: [u16; MAX_PATH],
        }

        pub type WNDENUMPROC = unsafe extern "system" fn(HWND, LPARAM) -> BOOL;

        #[link(name = "kernel32")]
        extern "system" {
            pub fn CreateToolhelp32Snapshot(dwFlags: DWORD, th32ProcessID: DWORD) -> HANDLE;
            pub fn Process32FirstW(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> BOOL;
            pub fn Process32NextW(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> BOOL;
            pub fn CloseHandle(hObject: HANDLE) -> BOOL;
        }

        pub type HDC = *mut core::ffi::c_void;
        pub type HGDIOBJ = *mut core::ffi::c_void;

        pub const SRCCOPY: DWORD = 0x00CC_0020;
        pub const CAPTUREBLT: DWORD = 0x4000_0000;
        pub const PW_RENDERFULLCONTENT: u32 = 0x0000_0002;
        pub const DIB_RGB_COLORS: u32 = 0;

        #[repr(C)]
        #[derive(Default)]
        pub struct RECT {
            pub left: i32,
            pub top: i32,
            pub right: i32,
            pub bottom: i32,
        }

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct POINT {
            pub x: i32,
            pub y: i32,
        }

        pub const GA_ROOT: u32 = 2;

        #[repr(C)]
        pub struct BITMAPINFOHEADER {
            pub biSize: DWORD,
            pub biWidth: i32,
            pub biHeight: i32,
            pub biPlanes: u16,
            pub biBitCount: u16,
            pub biCompression: DWORD,
            pub biSizeImage: DWORD,
            pub biXPelsPerMeter: i32,
            pub biYPelsPerMeter: i32,
            pub biClrUsed: DWORD,
            pub biClrImportant: DWORD,
        }

        #[repr(C)]
        pub struct BITMAPINFO {
            pub bmiHeader: BITMAPINFOHEADER,
            pub bmiColors: [DWORD; 1],
        }

        #[link(name = "gdi32")]
        extern "system" {
            pub fn CreateCompatibleDC(hdc: HDC) -> HDC;
            pub fn CreateCompatibleBitmap(hdc: HDC, cx: i32, cy: i32) -> HGDIOBJ;
            pub fn SelectObject(hdc: HDC, h: HGDIOBJ) -> HGDIOBJ;
            pub fn BitBlt(
                hdc: HDC,
                x: i32,
                y: i32,
                cx: i32,
                cy: i32,
                src: HDC,
                x1: i32,
                y1: i32,
                rop: DWORD,
            ) -> BOOL;
            pub fn GetDIBits(
                hdc: HDC,
                hbm: HGDIOBJ,
                start: u32,
                lines: u32,
                bits: *mut core::ffi::c_void,
                bmi: *mut BITMAPINFO,
                usage: u32,
            ) -> i32;
            pub fn DeleteObject(ho: HGDIOBJ) -> BOOL;
            pub fn DeleteDC(hdc: HDC) -> BOOL;
        }

        #[link(name = "user32")]
        extern "system" {
            pub fn GetWindowRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
            pub fn GetDC(hWnd: HWND) -> HDC;
            pub fn ReleaseDC(hWnd: HWND, hDC: HDC) -> i32;
            pub fn PrintWindow(hwnd: HWND, hdcBlt: HDC, nFlags: u32) -> BOOL;
            pub fn IsIconic(hWnd: HWND) -> BOOL;
            pub fn GetForegroundWindow() -> HWND;
            pub fn WindowFromPoint(point: POINT) -> HWND;
            pub fn GetAncestor(hwnd: HWND, flags: u32) -> HWND;
        }

        #[link(name = "user32")]
        extern "system" {
            pub fn EnumWindows(lpEnumFunc: WNDENUMPROC, lParam: LPARAM) -> BOOL;
            pub fn GetWindowThreadProcessId(hWnd: HWND, lpdwProcessId: *mut DWORD) -> DWORD;
            pub fn IsWindowVisible(hWnd: HWND) -> BOOL;
            pub fn GetWindowTextW(hWnd: HWND, lpString: *mut u16, nMaxCount: i32) -> i32;
            pub fn GetClassNameW(hWnd: HWND, lpClassName: *mut u16, nMaxCount: i32) -> i32;
        }
    }

    /// Every process on the machine, with its parent. One snapshot per call.
    pub fn processes() -> Vec<Proc> {
        let mut out = Vec::new();
        unsafe {
            let snapshot = ffi::CreateToolhelp32Snapshot(ffi::TH32CS_SNAPPROCESS, 0);
            if snapshot.is_null() || snapshot as isize == -1 {
                return out;
            }
            let mut entry: ffi::PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<ffi::PROCESSENTRY32W>() as u32;
            if ffi::Process32FirstW(snapshot, &mut entry) != 0 {
                loop {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|c| *c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    out.push(Proc {
                        pid: entry.th32ProcessID,
                        parent: entry.th32ParentProcessID,
                        name: String::from_utf16_lossy(&entry.szExeFile[..len]),
                    });
                    if ffi::Process32NextW(snapshot, &mut entry) == 0 {
                        break;
                    }
                }
            }
            ffi::CloseHandle(snapshot);
        }
        out
    }

    pub struct Seen {
        /// A normal, visible window with a title.
        pub window: Option<String>,
        /// That window's handle, for the proof screenshot.
        pub hwnd: Option<usize>,
        /// A dialog box, and its text. Nearly always an error.
        pub dialog: Option<String>,
    }

    struct Scan {
        wanted: Vec<u32>,
        window: Option<String>,
        hwnd: Option<usize>,
        dialog: Option<String>,
    }

    /// What is on screen belonging to this process or anything it started.
    pub fn look(root: u32) -> Option<Seen> {
        let all = processes();
        let mut wanted = vec![root];
        let mut index = 0;
        while index < wanted.len() {
            let parent = wanted[index];
            index += 1;
            for p in all.iter().filter(|p| p.parent == parent) {
                if !wanted.contains(&p.pid) {
                    wanted.push(p.pid);
                }
            }
        }

        let mut scan = Scan {
            wanted,
            window: None,
            hwnd: None,
            dialog: None,
        };
        unsafe {
            ffi::EnumWindows(enumerate, &mut scan as *mut Scan as ffi::LPARAM);
        }
        if scan.window.is_none() && scan.dialog.is_none() {
            return None;
        }
        Some(Seen {
            window: scan.window,
            hwnd: scan.hwnd,
            dialog: scan.dialog,
        })
    }

    unsafe extern "system" fn enumerate(hwnd: ffi::HWND, lparam: ffi::LPARAM) -> ffi::BOOL {
        let scan = &mut *(lparam as *mut Scan);

        if ffi::IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let mut pid: ffi::DWORD = 0;
        ffi::GetWindowThreadProcessId(hwnd, &mut pid);
        if !scan.wanted.contains(&pid) {
            return 1;
        }

        let title = text(|buf, len| ffi::GetWindowTextW(hwnd, buf, len));
        let class = text(|buf, len| ffi::GetClassNameW(hwnd, buf, len));

        // `#32770` is the class every standard Windows dialog box uses,
        // including the message box a game puts up when steam_api.dll is
        // missing or a runtime is not installed. A game's own window is never
        // this class, so it is a clean signal - and the title is usually the
        // whole explanation.
        if class == "#32770" {
            scan.dialog = Some(if title.trim().is_empty() {
                "an unnamed dialog".to_string()
            } else {
                title
            });
            return 0; // stop: nothing after this is more informative
        }

        // A window with no title is a splash, a tooltip or an engine's hidden
        // helper. Not evidence of anything.
        if !title.trim().is_empty() && scan.window.is_none() {
            scan.window = Some(title);
            scan.hwnd = Some(hwnd as usize);
        }
        1
    }

    /// Is every part of the window on screen the window itself?
    ///
    /// The game has to be the foreground window, and a grid of points across
    /// its rectangle must all belong to it. Only then is a copy of the screen
    /// at that rectangle a picture of the game and nothing else.
    unsafe fn unobstructed(hwnd: ffi::HWND, r: &ffi::RECT) -> bool {
        let root = ffi::GetAncestor(hwnd, ffi::GA_ROOT);
        let root = if root.is_null() { hwnd } else { root };
        let fg = ffi::GetForegroundWindow();
        if fg.is_null() || ffi::GetAncestor(fg, ffi::GA_ROOT) != root {
            return false;
        }
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        const STEPS: i32 = 6;
        for i in 0..=STEPS {
            for j in 0..=STEPS {
                let p = ffi::POINT {
                    x: r.left + 4 + (w - 8) * i / STEPS,
                    y: r.top + 4 + (h - 8) * j / STEPS,
                };
                let at = ffi::WindowFromPoint(p);
                if at.is_null() || ffi::GetAncestor(at, ffi::GA_ROOT) != root {
                    return false;
                }
            }
        }
        true
    }

    /// A picture of the window: BGRA, top row first, with its size.
    ///
    /// The window's own copy first (`PrintWindow`, which draws only that
    /// window, even when it is covered). A DirectX game often hands back a
    /// black copy of itself; then, and only when the game is in front and
    /// nothing else covers any part of it (`unobstructed`, checked before and
    /// after), what the screen shows where it is.
    ///
    /// Before 2026-10-16 the screen copy had no such check: when the game's
    /// own copy was black it took whatever was on screen at the game's
    /// rectangle - Forge, a chat, the desktop - and uploaded that as the
    /// release's "proof". `None` when nothing safe and not black is available,
    /// or the window is minimised.
    pub fn capture(hwnd: usize) -> Option<(u32, u32, Vec<u8>)> {
        unsafe {
            let hwnd = hwnd as ffi::HWND;
            if ffi::IsIconic(hwnd) != 0 {
                return None;
            }
            let mut r = ffi::RECT::default();
            if ffi::GetWindowRect(hwnd, &mut r) == 0 {
                return None;
            }
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            if !(64..=16384).contains(&w) || !(64..=16384).contains(&h) {
                return None;
            }
            let screen = ffi::GetDC(core::ptr::null_mut());
            if screen.is_null() {
                return None;
            }
            let mem = ffi::CreateCompatibleDC(screen);
            let bmp = ffi::CreateCompatibleBitmap(screen, w, h);
            let mut pixels = None;
            if !mem.is_null() && !bmp.is_null() {
                let old = ffi::SelectObject(mem, bmp);
                let printed = ffi::PrintWindow(hwnd, mem, ffi::PW_RENDERFULLCONTENT) != 0;
                ffi::SelectObject(mem, old);
                if printed {
                    pixels = read(screen, bmp, w, h).filter(|p| !mostly_black(p));
                }
                if pixels.is_none() && unobstructed(hwnd, &r) {
                    let old = ffi::SelectObject(mem, bmp);
                    ffi::BitBlt(
                        mem,
                        0,
                        0,
                        w,
                        h,
                        screen,
                        r.left,
                        r.top,
                        ffi::SRCCOPY | ffi::CAPTUREBLT,
                    );
                    ffi::SelectObject(mem, old);
                    // Still in front and uncovered after the copy, or it is thrown away.
                    if unobstructed(hwnd, &r) {
                        pixels = read(screen, bmp, w, h).filter(|p| !mostly_black(p));
                    }
                }
            }
            if !bmp.is_null() {
                ffi::DeleteObject(bmp);
            }
            if !mem.is_null() {
                ffi::DeleteDC(mem);
            }
            ffi::ReleaseDC(core::ptr::null_mut(), screen);
            pixels.map(|p| (w as u32, h as u32, p))
        }
    }

    /// The window's own drawing (`PrintWindow`, web views included), kept
    /// however dark it is. Our own window: never covered by anything that matters.
    pub fn print(hwnd: usize) -> Option<(u32, u32, Vec<u8>)> {
        unsafe {
            let hwnd = hwnd as ffi::HWND;
            if ffi::IsIconic(hwnd) != 0 {
                return None;
            }
            let mut r = ffi::RECT::default();
            if ffi::GetWindowRect(hwnd, &mut r) == 0 {
                return None;
            }
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            if !(64..=16384).contains(&w) || !(64..=16384).contains(&h) {
                return None;
            }
            let screen = ffi::GetDC(core::ptr::null_mut());
            if screen.is_null() {
                return None;
            }
            let mem = ffi::CreateCompatibleDC(screen);
            let bmp = ffi::CreateCompatibleBitmap(screen, w, h);
            let mut pixels = None;
            if !mem.is_null() && !bmp.is_null() {
                let old = ffi::SelectObject(mem, bmp);
                let printed = ffi::PrintWindow(hwnd, mem, ffi::PW_RENDERFULLCONTENT) != 0;
                ffi::SelectObject(mem, old);
                if printed {
                    pixels = read(screen, bmp, w, h);
                }
            }
            if !bmp.is_null() {
                ffi::DeleteObject(bmp);
            }
            if !mem.is_null() {
                ffi::DeleteDC(mem);
            }
            ffi::ReleaseDC(core::ptr::null_mut(), screen);
            pixels.map(|p| (w as u32, h as u32, p))
        }
    }

    /// The bitmap's pixels as top-down 32-bit BGRA. It must not be selected
    /// into a DC while this runs.
    unsafe fn read(dc: ffi::HDC, bmp: ffi::HGDIOBJ, w: i32, h: i32) -> Option<Vec<u8>> {
        let mut info = ffi::BITMAPINFO {
            bmiHeader: ffi::BITMAPINFOHEADER {
                biSize: core::mem::size_of::<ffi::BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [0],
        };
        let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
        let lines = ffi::GetDIBits(
            dc,
            bmp,
            0,
            h as u32,
            buf.as_mut_ptr().cast(),
            &mut info,
            ffi::DIB_RGB_COLORS,
        );
        (lines == h).then_some(buf)
    }

    /// Nearly nothing but black: a game that drew nothing, or drew it where GDI
    /// cannot see.
    fn mostly_black(bgra: &[u8]) -> bool {
        let mut sum = 0u64;
        let mut n = 0u64;
        for px in bgra.chunks_exact(4).step_by(97) {
            sum += px[0] as u64 + px[1] as u64 + px[2] as u64;
            n += 1;
        }
        n == 0 || sum / n < 12
    }

    /// Read a UTF-16 string out of one of the `...W` calls.
    fn text(mut get: impl FnMut(*mut u16, i32) -> i32) -> String {
        let mut buf = [0u16; 512];
        let len = get(buf.as_mut_ptr(), buf.len() as i32);
        if len <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn encodes_a_png() {
        let png = super::encode(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    }
}

//! Native CI capture of this process's visible popup, never another app's UI.
use super::{Tray, ffi::*, popup_window};
use std::{io, mem::size_of, path::Path, ptr::null_mut};
use std::{
    thread,
    time::{Duration, Instant},
};

#[repr(C)]
struct GuiThreadInfo {
    size: u32,
    flags: u32,
    active: Handle,
    focus: Handle,
    capture: Handle,
    menu_owner: Handle,
    move_size: Handle,
    caret: Handle,
    caret_rect: Rectangle,
}
#[link(name = "user32")]
unsafe extern "system" {
    fn GetForegroundWindow() -> Handle;
    fn AttachThreadInput(source: u32, target: u32, attach: i32) -> i32;
    fn GetGUIThreadInfo(thread: u32, information: *mut GuiThreadInfo) -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}
struct Attachment(u32, u32);
pub struct ProcessResources {
    pub working_set_bytes: usize,
    pub private_commit_bytes: usize,
    pub gdi_handles: u32,
    pub user_handles: u32,
}
#[repr(C)]
struct MemoryCounters {
    size: u32,
    faults: u32,
    peak_working: usize,
    working: usize,
    peak_paged: usize,
    paged: usize,
    peak_nonpaged: usize,
    nonpaged: usize,
    pagefile: usize,
    peak_pagefile: usize,
    private: usize,
}
#[link(name = "psapi")]
unsafe extern "system" {
    fn GetProcessMemoryInfo(process: Handle, counters: *mut MemoryCounters, size: u32) -> i32;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn GetGuiResources(process: Handle, flags: u32) -> u32;
}
pub fn process_resources() -> io::Result<ProcessResources> {
    // SAFETY: the current-process pseudo handle is borrowed; a correctly sized,
    // initialized SDK-layout buffer lives through this synchronous read.
    unsafe {
        let process = -1_isize as Handle;
        let mut counters: MemoryCounters = std::mem::zeroed();
        let size = size_of::<MemoryCounters>() as u32;
        counters.size = size;
        if GetProcessMemoryInfo(process, &mut counters, size) == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(ProcessResources {
            working_set_bytes: counters.working,
            private_commit_bytes: counters.private,
            gdi_handles: GetGuiResources(process, 0),
            user_handles: GetGuiResources(process, 1),
        })
    }
}
impl Drop for Attachment {
    fn drop(&mut self) {
        unsafe {
            AttachThreadInput(self.0, self.1, 0);
        }
    }
}
fn disposable_desktop() -> io::Result<()> {
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || std::env::var_os("RUNNER_TEMP").is_none()
    {
        return Err(io::Error::other(
            "keyboard acceptance requires a disposable GitHub desktop",
        ));
    }
    Ok(())
}
/// Acquire foreground before opening the menu, on its creator thread. Input
/// queues are detached before returning; no owner desktop is used by this test.
pub fn focus(tray: &Tray) -> io::Result<()> {
    disposable_desktop()?;
    // SAFETY: Tray's !Send ownership keeps this call on the window's creator
    // thread. Borrowed foreground handles and attached queues live only here.
    unsafe {
        let current = GetCurrentThreadId();
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), null_mut());
        let _attachment = if foreground != 0 && foreground != current {
            if AttachThreadInput(current, foreground, 1) == 0 {
                return Err(io::Error::last_os_error());
            }
            Some(Attachment(current, foreground))
        } else {
            None
        };
        SetForegroundWindow(tray.window);
        SetFocus(tray.window);
        let mut owner = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), &mut owner);
        if owner != std::process::id() {
            return Err(io::Error::other(
                "native test window did not acquire foreground",
            ));
        }
    }
    Ok(())
}

pub fn capture_popup(path: &Path) -> io::Result<bool> {
    // SAFETY: only integer handles from Win32 are traversed. A window must belong
    // to this process and have bounded dimensions before allocating a bitmap.
    unsafe {
        let Some(window) = popup_window(None) else {
            return Ok(false);
        };
        // A menu HWND exists before native layout/presentation completes. Wait
        // for real visible item rectangles, then capture its final geometry.
        let mut info: MenuBarInfo = std::mem::zeroed();
        info.size = size_of::<MenuBarInfo>() as u32;
        if GetMenuBarInfo(window, -4, 0, &mut info) == 0 || info.menu.is_null() {
            return Ok(false);
        }
        let count = GetMenuItemCount(info.menu);
        let mut item: Rectangle = std::mem::zeroed();
        if count <= 0
            || GetMenuItemRect(window, info.menu, (count - 1) as u32, &mut item) == 0
            || item.bottom <= item.top
        {
            return Ok(false);
        }
        capture_window(window, path)
    }
}
fn dashboard_window() -> Option<Handle> {
    // SAFETY: borrowed handle is accepted only for our own visible window.
    unsafe {
        let window = FindWindowW(
            super::wide("org.nddev.systeminfo.analysis").as_ptr(),
            std::ptr::null(),
        );
        let mut owner = 0;
        if window.is_null() || IsWindowVisible(window) == 0 {
            return None;
        }
        GetWindowThreadProcessId(window, &mut owner);
        (owner == std::process::id()).then_some(window)
    }
}
pub fn dashboard_visible() -> bool {
    dashboard_window().is_some()
}
pub fn capture_dashboard(path: &Path) -> io::Result<bool> {
    let Some(window) = dashboard_window() else {
        return Ok(false);
    };
    capture_window(window, path)
}
/// Read standard label names, which Windows exposes to assistive clients.
pub fn dashboard_labels() -> io::Result<Vec<String>> {
    let Some(window) = dashboard_window() else {
        return Ok(Vec::new());
    };
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetWindowTextW(window: Handle, text: *mut u16, count: i32) -> i32;
    }
    let mut labels = Vec::new();
    let mut after = null_mut();
    // SAFETY: enumerate only standard child labels of our verified own window.
    // The bounded UTF-16 buffer lives through each synchronous native read.
    unsafe {
        for _ in 0..512 {
            let label = FindWindowExW(
                window,
                after,
                super::wide("STATIC").as_ptr(),
                std::ptr::null(),
            );
            if label.is_null() {
                return Ok(labels);
            }
            let mut text = vec![0_u16; 16385];
            let length = GetWindowTextW(label, text.as_mut_ptr(), text.len() as i32);
            if length < 0 {
                return Err(io::Error::last_os_error());
            }
            labels.push(String::from_utf16_lossy(&text[..length as usize]));
            after = label;
        }
    }
    Err(io::Error::other("native label enumeration exceeded bounds"))
}
pub fn click_dashboard(id: u32) -> io::Result<bool> {
    disposable_desktop()?;
    let Some(window) = dashboard_window() else {
        return Ok(false);
    };
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDlgItem(window: Handle, id: i32) -> Handle;
        fn SendMessageW(window: Handle, message: u32, wparam: usize, lparam: isize) -> isize;
    }
    // SAFETY: the standard button belongs to the verified owned window. Its
    // normal BM_CLICK path generates the same BN_CLICKED command as input.
    unsafe {
        let button = GetDlgItem(window, id as i32);
        if button.is_null() {
            return Ok(false);
        }
        SendMessageW(button, 0x00f5, 0, 0);
        Ok(true)
    }
}
fn capture_window(window: Handle, path: &Path) -> io::Result<bool> {
    // SAFETY: callers verified that this visible window belongs to this process;
    // bitmap dimensions are bounded before allocating and GDI ownership balances.
    unsafe {
        UpdateWindow(window);
        DwmFlush();
        let mut rect = Rectangle {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(window, &mut rect) == 0 {
            return Err(io::Error::last_os_error());
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width < 16 || height < 16 {
            return Ok(false);
        }
        if width > 2048 || height > 2048 {
            return Err(io::Error::other("popup exceeds capture bounds"));
        }
        let screen = GetDC(null_mut());
        if screen.is_null() {
            return Err(io::Error::last_os_error());
        }
        let dc = CreateCompatibleDC(screen);
        if dc.is_null() {
            ReleaseDC(null_mut(), screen);
            return Err(io::Error::last_os_error());
        }
        let header = BitmapHeader {
            size: size_of::<BitmapHeader>() as u32,
            width,
            height: -height,
            planes: 1,
            bits: 32,
            compression: 0,
            image_size: 0,
            x_resolution: 0,
            y_resolution: 0,
            used_colors: 0,
            important_colors: 0,
        };
        let mut pointer = null_mut();
        let bitmap = CreateDIBSection(dc, &header, 0, &mut pointer, null_mut(), 0);
        if bitmap.is_null() || pointer.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            DeleteDC(dc);
            ReleaseDC(null_mut(), screen);
            return Err(io::Error::last_os_error());
        }
        let previous = SelectObject(dc, bitmap);
        let copied = BitBlt(
            dc,
            0,
            0,
            width,
            height,
            screen,
            rect.left,
            rect.top,
            0x00cc0020 | 0x40000000,
        );
        GdiFlush();
        let pixels = if copied != 0 {
            // The initialized top-down 32-bit DIB contains exactly width*height
            // pixels. Copy before restoring/deleting the selected GDI objects.
            std::slice::from_raw_parts(pointer.cast::<u8>(), (width * height * 4) as usize).to_vec()
        } else {
            Vec::new()
        };
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        ReleaseDC(null_mut(), screen);
        if copied == 0 {
            return Err(io::Error::last_os_error());
        }
        save_png(path, width as u32, height as u32, &pixels)?;
        Ok(true)
    }
}
/// Read the actual native tree while its modal loop is running. This acceptance
/// surface never returns a model's cached strings in place of OS state.
pub fn popup_entries() -> io::Result<Option<Vec<(String, bool)>>> {
    let Some(window) = popup_window(None) else {
        return Ok(None);
    };
    // SAFETY: borrowed menu belongs to this process; each bounded text buffer
    // lives through the call and Win32 reports its initialized UTF-16 length.
    unsafe {
        let mut info: MenuBarInfo = std::mem::zeroed();
        info.size = size_of::<MenuBarInfo>() as u32;
        if GetMenuBarInfo(window, -4, 0, &mut info) == 0 {
            return Ok(None);
        }
        let count = GetMenuItemCount(info.menu);
        if !(1..=128).contains(&count) {
            return Err(io::Error::other("invalid native menu size"));
        }
        let mut entries = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut text = [0u16; 1024];
            let length = GetMenuStringW(
                info.menu,
                index as u32,
                text.as_mut_ptr(),
                text.len() as i32,
                0x400,
            );
            let state = GetMenuState(info.menu, index as u32, 0x400);
            if state == u32::MAX || length < 0 || length as usize >= text.len() - 1 {
                return Err(io::Error::other("native menu item unavailable"));
            }
            entries.push((
                String::from_utf16_lossy(&text[..length as usize]),
                state & 3 == 0,
            ));
        }
        Ok(Some(entries))
    }
}

/// Deliver Up/Enter only to this process's native popup, never to another
/// application's foreground window. The caller must observe the real command.
pub fn choose_last_action() -> io::Result<()> {
    disposable_desktop()?;
    popup_window(None).ok_or_else(|| io::Error::other("owned popup missing"))?;
    // SAFETY: input is posted only after checking this disposable process owns
    // foreground and the menu. All arguments are values, with no Rust pointers.
    unsafe {
        let mut owner = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), &mut owner);
        if owner != std::process::id() {
            return Err(io::Error::other(
                "keyboard target is not our foreground window",
            ));
        }
        let mut info: GuiThreadInfo = std::mem::zeroed();
        info.size = size_of::<GuiThreadInfo>() as u32;
        if GetGUIThreadInfo(0, &mut info) == 0 {
            return Err(io::Error::last_os_error());
        }
        eprintln!(
            "native input state: flags={:#x}, active={:?}, focus={:?}, menu_owner={:?}",
            info.flags, info.active, info.focus, info.menu_owner
        );
        let target = info.menu_owner;
        if target.is_null() {
            return Err(io::Error::other("native menu owner missing"));
        }
        // Address the owning GUI queue. Hosted runners may have a disconnected
        // input desktop; the modal menu still consumes real WM_KEYDOWN/UP
        // messages on its owner thread, rather than on the popup HWND.
        if PostMessageW(target, 0x100, 0x26, 0x01480001) == 0
            || PostMessageW(target, 0x101, 0x26, 0xc1480001_u32 as isize) == 0
        {
            return Err(io::Error::last_os_error());
        }
        // Observe the real menu selection before Enter. This distinguishes an
        // input-routing failure from a lost command callback without guessing
        // how long this runner takes to consume keyboard input.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let window = popup_window(None)
                .ok_or_else(|| io::Error::other("menu closed before selection"))?;
            let mut menu: MenuBarInfo = std::mem::zeroed();
            menu.size = size_of::<MenuBarInfo>() as u32;
            if GetMenuBarInfo(window, -4, 0, &mut menu) == 0 {
                return Err(io::Error::last_os_error());
            }
            let count = GetMenuItemCount(menu.menu);
            let state = GetMenuState(menu.menu, (count - 1) as u32, 0x400);
            if state & 0x80 != 0 {
                break;
            }
            if Instant::now() >= deadline {
                return Err(io::Error::other(format!(
                    "Up did not select the last native action: state={state:#x}, GUI flags={:#x}, focus={:?}",
                    info.flags, info.focus
                )));
            }
            thread::sleep(Duration::from_millis(10));
        }
        if PostMessageW(target, 0x100, 0x0d, 0x001c0001) == 0
            || PostMessageW(target, 0x101, 0x0d, 0xc01c0001_u32 as isize) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
fn save_png(path: &Path, width: u32, height: u32, bgra: &[u8]) -> io::Result<()> {
    // PNG's uncompressed DEFLATE blocks keep this test-only encoder independent
    // of a resident image/graphics library. Every row uses PNG filter type zero.
    let mut raw = Vec::with_capacity(bgra.len() + height as usize);
    for row in bgra.chunks_exact(width as usize * 4) {
        raw.push(0);
        for pixel in row.as_chunks::<4>().0 {
            raw.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    let mut compressed = vec![0x78, 0x01];
    let mut blocks = raw.chunks(u16::MAX as usize).peekable();
    while let Some(block) = blocks.next() {
        compressed.push(u8::from(blocks.peek().is_none()));
        let length = block.len() as u16;
        compressed.extend_from_slice(&length.to_le_bytes());
        compressed.extend_from_slice(&(!length).to_le_bytes());
        compressed.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    compressed.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut image = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::from(width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    for (kind, content) in [
        (b"IHDR", header),
        (b"IDAT", compressed),
        (b"IEND", Vec::new()),
    ] {
        image.extend_from_slice(&(content.len() as u32).to_be_bytes());
        image.extend_from_slice(kind);
        image.extend_from_slice(&content);
        let mut crc = u32::MAX;
        for byte in kind.iter().chain(content.iter()) {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        image.extend_from_slice(&(!crc).to_be_bytes());
    }
    std::fs::write(path, image)
}

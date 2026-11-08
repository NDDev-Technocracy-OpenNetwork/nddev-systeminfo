//! Minimal Win32 ABI declarations used by the approved tray boundary. Layouts
//! follow the Windows SDK headers; pointers never escape the safe tray module.
use std::ffi::c_void;
pub(super) type Handle = *mut c_void;
pub(super) type WndProc = unsafe extern "system" fn(Handle, u32, usize, isize) -> isize;
#[repr(C)]
#[derive(Default)]
pub(super) struct Point {
    pub x: i32,
    pub y: i32,
}
#[repr(C)]
pub(super) struct Message {
    pub window: Handle,
    pub message: u32,
    pub wparam: usize,
    pub lparam: isize,
    pub time: u32,
    pub point: Point,
    pub private: u32,
}
#[repr(C)]
pub(super) struct WindowClass {
    pub style: u32,
    pub procedure: Option<WndProc>,
    pub class_extra: i32,
    pub window_extra: i32,
    pub instance: Handle,
    pub icon: Handle,
    pub cursor: Handle,
    pub background: Handle,
    pub menu_name: *const u16,
    pub class_name: *const u16,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Guid {
    pub first: u32,
    pub second: u16,
    pub third: u16,
    pub bytes: [u8; 8],
}
#[repr(C)]
pub(super) struct NotifyIcon {
    pub size: u32,
    pub window: Handle,
    pub id: u32,
    pub flags: u32,
    pub callback: u32,
    pub icon: Handle,
    pub tip: [u16; 128],
    pub state: u32,
    pub state_mask: u32,
    pub info: [u16; 256],
    pub version: u32,
    pub info_title: [u16; 64],
    pub info_flags: u32,
    pub guid: Guid,
    pub balloon_icon: Handle,
}
#[repr(C)]
pub(super) struct IconInfo {
    pub icon: i32,
    pub x_hotspot: u32,
    pub y_hotspot: u32,
    pub mask: Handle,
    pub color: Handle,
}
#[repr(C)]
pub(super) struct BitmapHeader {
    pub size: u32,
    pub width: i32,
    pub height: i32,
    pub planes: u16,
    pub bits: u16,
    pub compression: u32,
    pub image_size: u32,
    pub x_resolution: i32,
    pub y_resolution: i32,
    pub used_colors: u32,
    pub important_colors: u32,
}
#[repr(C)]
pub(super) struct HighContrast {
    pub size: u32,
    pub flags: u32,
    pub scheme: *mut u16,
}

#[link(name = "user32")]
unsafe extern "system" {
    pub(super) fn RegisterClassW(class: *const WindowClass) -> u16;
    pub(super) fn UnregisterClassW(name: *const u16, instance: Handle) -> i32;
    pub(super) fn CreateWindowExW(
        style: u32,
        class: *const u16,
        title: *const u16,
        window_style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Handle,
        menu: Handle,
        instance: Handle,
        parameter: *mut c_void,
    ) -> Handle;
    pub(super) fn DestroyWindow(window: Handle) -> i32;
    pub(super) fn DefWindowProcW(
        window: Handle,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize;
    pub(super) fn PostMessageW(window: Handle, message: u32, wparam: usize, lparam: isize) -> i32;
    pub(super) fn PostQuitMessage(exit_code: i32);
    pub(super) fn GetMessageW(message: *mut Message, window: Handle, min: u32, max: u32) -> i32;
    pub(super) fn DispatchMessageW(message: *const Message) -> isize;
    pub(super) fn TranslateMessage(message: *const Message) -> i32;
    pub(super) fn RegisterWindowMessageW(name: *const u16) -> u32;
    pub(super) fn CreatePopupMenu() -> Handle;
    pub(super) fn AppendMenuW(menu: Handle, flags: u32, id: usize, title: *const u16) -> i32;
    pub(super) fn DestroyMenu(menu: Handle) -> i32;
    pub(super) fn TrackPopupMenuEx(
        menu: Handle,
        flags: u32,
        x: i32,
        y: i32,
        window: Handle,
        params: *const c_void,
    ) -> u32;
    pub(super) fn GetCursorPos(point: *mut Point) -> i32;
    pub(super) fn SetForegroundWindow(window: Handle) -> i32;
    pub(super) fn EndMenu() -> i32;
    pub(super) fn CreateIconIndirect(info: *const IconInfo) -> Handle;
    pub(super) fn DestroyIcon(icon: Handle) -> i32;
    pub(super) fn GetSysColor(index: i32) -> u32;
    pub(super) fn GetDpiForWindow(window: Handle) -> u32;
    pub(super) fn SetProcessDpiAwarenessContext(context: Handle) -> i32;
    pub(super) fn SystemParametersInfoW(
        action: u32,
        parameter: u32,
        value: *mut c_void,
        flags: u32,
    ) -> i32;
    pub(super) fn OpenClipboard(window: Handle) -> i32;
    pub(super) fn EmptyClipboard() -> i32;
    pub(super) fn SetClipboardData(format: u32, memory: Handle) -> Handle;
    pub(super) fn CloseClipboard() -> i32;
}
#[link(name = "shell32")]
unsafe extern "system" {
    pub(super) fn Shell_NotifyIconW(message: u32, data: *const NotifyIcon) -> i32;
}
#[link(name = "gdi32")]
unsafe extern "system" {
    pub(super) fn CreateDIBSection(
        dc: Handle,
        info: *const BitmapHeader,
        usage: u32,
        bits: *mut *mut c_void,
        section: Handle,
        offset: u32,
    ) -> Handle;
    pub(super) fn CreateBitmap(
        width: i32,
        height: i32,
        planes: u32,
        bits: u32,
        data: *const c_void,
    ) -> Handle;
    pub(super) fn DeleteObject(object: Handle) -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    pub(super) fn GetModuleHandleW(name: *const u16) -> Handle;
    pub(super) fn CreateMutexW(attributes: *const c_void, owner: i32, name: *const u16) -> Handle;
    pub(super) fn GetLastError() -> u32;
    pub(super) fn CloseHandle(handle: Handle) -> i32;
    pub(super) fn GetUserDefaultLocaleName(name: *mut u16, length: i32) -> i32;
    pub(super) fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
    pub(super) fn GlobalLock(memory: Handle) -> *mut c_void;
    pub(super) fn GlobalUnlock(memory: Handle) -> i32;
    pub(super) fn GlobalFree(memory: Handle) -> Handle;
    pub(super) fn FindFirstChangeNotificationW(
        path: *const u16,
        subtree: i32,
        filter: u32,
    ) -> Handle;
    pub(super) fn FindNextChangeNotification(handle: Handle) -> i32;
    pub(super) fn FindCloseChangeNotification(handle: Handle) -> i32;
    pub(super) fn CreateEventW(
        attributes: *const c_void,
        manual: i32,
        initial: i32,
        name: *const u16,
    ) -> Handle;
    pub(super) fn SetEvent(event: Handle) -> i32;
    pub(super) fn WaitForMultipleObjects(
        count: u32,
        handles: *const Handle,
        all: i32,
        timeout: u32,
    ) -> u32;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    pub(super) fn RegGetValueW(
        key: Handle,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> i32;
}

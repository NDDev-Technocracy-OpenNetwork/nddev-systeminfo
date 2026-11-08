//! Safe ownership boundary for native notification icons and menus. No pointer
//! from a window procedure refers to Rust state. The creator thread owns every
//! window/menu/icon; other threads post wakeups through a bounded latest value.
#[cfg(feature = "ui-acceptance")]
pub mod acceptance;
mod ffi;
use ffi::*;
use std::{
    cell::Cell,
    io,
    marker::PhantomData,
    mem::{size_of, zeroed},
    path::Path,
    ptr::{null, null_mut},
    rc::Rc,
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

const CALLBACK: u32 = 0x8001;
const UPDATE: u32 = 0x8002;
const OPEN: u32 = 0x8003;
const CONFIG_CHANGED: u32 = 0x8004;
const THEME_CHANGED: u32 = 0x8005;
const CANCEL_MENU: u32 = 0x8006;
const CLOSE: u32 = 0x0010;
thread_local! { static DEFERRED: Cell<u8> = const { Cell::new(0) }; }
static TASKBAR_MESSAGE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[derive(Clone, PartialEq, Eq)]
pub enum Entry {
    Label(String),
    Separator,
    Action {
        id: u32,
        title: String,
        enabled: bool,
        checked: bool,
    },
    Submenu {
        title: String,
        entries: Vec<Entry>,
    },
}
#[derive(Clone, PartialEq, Eq)]
pub struct Model {
    pub tooltip: String,
    pub entries: Vec<Entry>,
    pub rtl: bool,
    /// Premultiplied BGRA from the released vector. Square, <=128px.
    pub icon: Vec<u8>,
    pub icon_side: u32,
}
pub enum Event {
    Command(u32),
    ConfigurationChanged,
    AppearanceChanged,
}
struct Shared {
    window: usize,
    model: Model,
    pending: bool,
}
#[derive(Clone)]
pub struct Proxy(Arc<Mutex<Shared>>);
impl Proxy {
    pub fn update(&self, model: Model) {
        let mut state = self.0.lock().unwrap();
        if state.model == model {
            return;
        }
        state.model = model;
        if state.window != 0 && !state.pending {
            state.pending = true;
            // SAFETY: window stays owned while this mutex is locked. Payloads
            // are integer wakeups only; no cross-thread pointers or allocations.
            unsafe {
                PostMessageW(state.window as Handle, UPDATE, 0, 0);
            }
        }
    }
    pub fn close(&self) {
        self.post(CLOSE);
    }
    pub fn open_menu(&self) {
        self.post(OPEN);
    }
    pub fn cancel_menu(&self) {
        self.post(CANCEL_MENU);
    }
    pub fn watch_config(&self, path: &Path) -> io::Result<Watcher> {
        Watcher::new(path, self.clone())
    }
    fn post(&self, message: u32) {
        let state = self.0.lock().unwrap();
        if state.window != 0 {
            // SAFETY: window ownership is synchronized by the same mutex as Drop.
            unsafe {
                PostMessageW(state.window as Handle, message, 0, 0);
            }
        }
    }
}

pub struct Tray {
    window: Handle,
    instance: Handle,
    class: Vec<u16>,
    singleton: Handle,
    icon: Handle,
    pixels: Vec<u8>,
    data: NotifyIcon,
    taskbar_created: u32,
    proxy: Proxy,
    _creator_thread: PhantomData<Rc<()>>,
}
impl Tray {
    pub fn new(identity: &str, model: Model) -> io::Result<Option<Self>> {
        let name = wide(&format!("Local\\{identity}"));
        // SAFETY: NUL-terminated names live through each synchronous call. The
        // returned owned mutex is closed exactly once, including duplicate launch.
        let singleton = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        if singleton.is_null() {
            return Err(io::Error::last_os_error());
        }
        if unsafe { GetLastError() } == 183 {
            unsafe {
                CloseHandle(singleton);
            }
            return Ok(None);
        }
        let class = wide(identity);
        // SAFETY: instance is borrowed; class registration is bound to our module.
        // No Rust pointer is installed in class or window userdata.
        let instance = unsafe { GetModuleHandleW(null()) };
        let definition = WindowClass {
            style: 0,
            procedure: Some(procedure),
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: null_mut(),
            cursor: null_mut(),
            background: null_mut(),
            menu_name: null(),
            class_name: class.as_ptr(),
        };
        unsafe {
            SetProcessDpiAwarenessContext(-4_isize as Handle);
        }
        if unsafe { RegisterClassW(&definition) } == 0 {
            unsafe {
                CloseHandle(singleton);
            }
            return Err(io::Error::last_os_error());
        }
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            )
        };
        if window.is_null() {
            unsafe {
                UnregisterClassW(class.as_ptr(), instance);
                CloseHandle(singleton);
            }
            return Err(io::Error::last_os_error());
        }
        // SAFETY: all-zero is valid for this C POD; cbSize declares the full ABI.
        let mut data: NotifyIcon = unsafe { zeroed() };
        data.size = size_of::<NotifyIcon>() as u32;
        data.window = window;
        data.id = 1;
        data.flags = 1 | 2 | 4 | 128; // message, icon, tooltip, standard tooltip in v4
        data.callback = CALLBACK;
        data.version = 4;
        let proxy = Proxy(Arc::new(Mutex::new(Shared {
            window: window as usize,
            model,
            pending: false,
        })));
        let taskbar_created = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
        TASKBAR_MESSAGE.store(taskbar_created, std::sync::atomic::Ordering::Relaxed);
        let mut tray = Self {
            window,
            instance,
            class,
            singleton,
            icon: null_mut(),
            pixels: Vec::new(),
            data,
            taskbar_created,
            proxy,
            _creator_thread: PhantomData,
        };
        tray.refresh()?;
        tray.add_icon()?;
        Ok(Some(tray))
    }
    pub fn proxy(&self) -> Proxy {
        self.proxy.clone()
    }
    fn add_icon(&self) -> io::Result<()> {
        // SAFETY: full-size initialized POD and valid owned icon/window. Version
        // is set after every ADD, including Explorer restart (Microsoft contract).
        if unsafe { Shell_NotifyIconW(0, &self.data) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { Shell_NotifyIconW(4, &self.data) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    fn refresh(&mut self) -> io::Result<()> {
        let model = {
            let mut state = self.proxy.0.lock().unwrap();
            state.pending = false;
            state.model.clone()
        };
        let mut changed = false;
        let old = if self.pixels != model.icon {
            let icon = create_icon(&model)?;
            let old = std::mem::replace(&mut self.icon, icon);
            self.data.icon = icon;
            self.pixels = model.icon;
            changed = true;
            old
        } else {
            null_mut()
        };
        let text: Vec<_> = model.tooltip.encode_utf16().take(127).collect();
        let mut tip = [0; 128];
        tip[..text.len()].copy_from_slice(&text);
        if self.data.tip != tip {
            self.data.tip = tip;
            changed = true;
        }
        // SAFETY: notification area copies the supplied icon/tooltip during the
        // call. The old icon is destroyed only after the replacement is installed.
        unsafe {
            if changed {
                Shell_NotifyIconW(1, &self.data);
            }
            if !old.is_null() {
                DestroyIcon(old);
            }
        }
        Ok(())
    }
    pub fn run(mut self, mut on_event: impl FnMut(Event, &Proxy)) -> io::Result<()> {
        loop {
            // SAFETY: GetMessage initializes this POD on success. This thread
            // owns the window and dispatches until WM_QUIT; no polling timer.
            let mut message: Message = unsafe { zeroed() };
            match unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } {
                -1 => return Err(io::Error::last_os_error()),
                0 => return Ok(()),
                _ => {}
            }
            match message.message {
                UPDATE => {
                    self.refresh()?;
                }
                OPEN => {
                    on_event(Event::ConfigurationChanged, &self.proxy);
                    self.refresh()?;
                    if let Some(command) = self.menu()? {
                        on_event(Event::Command(command), &self.proxy);
                    }
                    // Native popup tracking runs a nested message loop. Its
                    // procedure records our wakeups instead of dropping them.
                    let deferred = DEFERRED.with(|value| value.replace(0));
                    if deferred & 2 != 0 {
                        on_event(Event::ConfigurationChanged, &self.proxy);
                    }
                    if deferred & 4 != 0 {
                        on_event(Event::AppearanceChanged, &self.proxy);
                    }
                    self.refresh()?;
                    if deferred & 8 != 0 {
                        self.add_icon()?;
                    }
                    unsafe {
                        Shell_NotifyIconW(3, &self.data);
                    }
                }
                CONFIG_CHANGED => on_event(Event::ConfigurationChanged, &self.proxy),
                THEME_CHANGED => on_event(Event::AppearanceChanged, &self.proxy),
                value if value == self.taskbar_created => {
                    self.add_icon()?;
                }
                _ => unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                },
            }
        }
    }
    fn menu(&self) -> io::Result<Option<u32>> {
        let model = self.proxy.0.lock().unwrap().model.clone();
        let menu = create_menu(&model.entries)?;
        let mut point = Point::default();
        // SAFETY: popup handle belongs to this scope. Foreground ownership is
        // required for dismiss-on-outside-click; coordinates come from Win32.
        let command = unsafe {
            GetCursorPos(&mut point);
            SetForegroundWindow(self.window);
            let command = TrackPopupMenuEx(
                menu.0,
                0x100 | 2 | if model.rtl { 0x8000 } else { 0 },
                point.x,
                point.y,
                self.window,
                null(),
            );
            PostMessageW(self.window, 0, 0, 0);
            command
        };
        Ok((command != 0).then_some(command))
    }
    pub fn watch_config(&self, path: &Path) -> io::Result<Watcher> {
        Watcher::new(path, self.proxy.clone())
    }
    pub fn dpi(&self) -> u32 {
        // SAFETY: valid window on its creator thread; Windows 10+ is required.
        unsafe { GetDpiForWindow(self.window) }.max(96)
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        self.proxy.0.lock().unwrap().window = 0;
        // SAFETY: resources are exclusively owned and released in reverse order
        // on the creator thread. Proxy can no longer enqueue after the lock above.
        unsafe {
            Shell_NotifyIconW(2, &self.data);
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
            }
            DestroyWindow(self.window);
            UnregisterClassW(self.class.as_ptr(), self.instance);
            CloseHandle(self.singleton);
        }
    }
}
unsafe extern "system" fn procedure(
    window: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    // SAFETY: Win32 invokes this callback for a live window. All posted data is
    // integer-only; no panic, userdata dereference, allocation or Rust callback.
    unsafe {
        match message {
            UPDATE | CONFIG_CHANGED | THEME_CHANGED => {
                let flag = if message == UPDATE {
                    1
                } else if message == CONFIG_CHANGED {
                    2
                } else {
                    4
                };
                DEFERRED.with(|value| value.set(value.get() | flag));
                0
            }
            value
                if value != 0
                    && value == TASKBAR_MESSAGE.load(std::sync::atomic::Ordering::Relaxed) =>
            {
                DEFERRED.with(|value| value.set(value.get() | 8));
                0
            }
            CANCEL_MENU => {
                EndMenu();
                0
            }
            CLOSE => {
                PostQuitMessage(0);
                0
            }
            CALLBACK => {
                if matches!((lparam as u32) & 0xffff, 0x0205 | 0x007b | 0x0400 | 0x0401) {
                    PostMessageW(window, OPEN, 0, 0);
                }
                0
            }
            0x001a | 0x031a | 0x02e0 => {
                PostMessageW(window, THEME_CHANGED, 0, 0);
                0
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}
struct Menu(Handle);
impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            DestroyMenu(self.0);
        }
    }
}
fn create_menu(entries: &[Entry]) -> io::Result<Menu> {
    // SAFETY: each menu is owned by its Menu guard until parent adoption, when
    // Win32 recursively owns/destroys the submenu. Strings are copied by Win32.
    let handle = unsafe { CreatePopupMenu() };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let menu = Menu(handle);
    for entry in entries {
        let (flags, id, text, child) = match entry {
            Entry::Separator => (0x800, 0, String::new(), None),
            Entry::Label(text) => (0, 0, text.clone(), None),
            Entry::Action {
                id,
                title,
                enabled,
                checked,
            } => (
                (if *enabled { 0 } else { 0x2 }) | if *checked { 8 } else { 0 },
                *id as usize,
                title.clone(),
                None,
            ),
            Entry::Submenu { title, entries } => {
                let child = create_menu(entries)?;
                (0x10, child.0 as usize, title.clone(), Some(child))
            }
        };
        let result =
            unsafe { AppendMenuW(menu.0, flags, id, wide(&text.replace('&', "&&")).as_ptr()) };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        if let Some(child) = child {
            std::mem::forget(child);
        }
    }
    Ok(menu)
}
fn create_icon(model: &Model) -> io::Result<Handle> {
    let side = model.icon_side;
    if !(16..=128).contains(&side)
        || !side.is_multiple_of(16)
        || model.icon.len() != (side * side * 4) as usize
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid tray icon pixels",
        ));
    }
    let header = BitmapHeader {
        size: size_of::<BitmapHeader>() as u32,
        width: side as i32,
        height: -(side as i32),
        planes: 1,
        bits: 32,
        compression: 0,
        image_size: 0,
        x_resolution: 0,
        y_resolution: 0,
        used_colors: 0,
        important_colors: 0,
    };
    let mut bits = null_mut();
    // SAFETY: BI_RGB/32bpp uses only the 40-byte header, no color table. The
    // returned buffer has exactly side*side*4 writable bytes. CreateIconIndirect
    // copies both bitmaps; both GDI objects are then deleted on every path.
    unsafe {
        let bitmap = CreateDIBSection(null_mut(), &header, 0, &mut bits, null_mut(), 0);
        if bitmap.is_null() {
            return Err(io::Error::last_os_error());
        }
        if bits.is_null() {
            DeleteObject(bitmap);
            return Err(io::Error::other("DIB buffer absent"));
        }
        std::ptr::copy_nonoverlapping(model.icon.as_ptr(), bits.cast(), model.icon.len());
        let mask = CreateBitmap(side as i32, side as i32, 1, 1, null());
        let icon = if mask.is_null() {
            null_mut()
        } else {
            CreateIconIndirect(&IconInfo {
                icon: 1,
                x_hotspot: 0,
                y_hotspot: 0,
                mask,
                color: bitmap,
            })
        };
        DeleteObject(bitmap);
        if !mask.is_null() {
            DeleteObject(mask);
        }
        if icon.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(icon)
        }
    }
}
pub fn system_appearance() -> (bool, bool, [u8; 3]) {
    // SAFETY: HIGHCONTRAST cbSize and writable memory follow the SDK ABI. OS
    // colors are value returns; no resource needs releasing.
    unsafe {
        let mut contrast = HighContrast {
            size: size_of::<HighContrast>() as u32,
            flags: 0,
            scheme: null_mut(),
        };
        SystemParametersInfoW(
            0x0042,
            contrast.size,
            (&mut contrast as *mut HighContrast).cast(),
            0,
        );
        let color = GetSysColor(8); // COLOR_WINDOWTEXT, an accessibility-safe icon
        let background = GetSysColor(5);
        let rgb = [
            (color & 255) as u8,
            ((color >> 8) & 255) as u8,
            ((color >> 16) & 255) as u8,
        ];
        let mut dark =
            (background & 255) + ((background >> 8) & 255) + ((background >> 16) & 255) < 384;
        let mut light: u32 = 1;
        let mut length = size_of::<u32>() as u32;
        if RegGetValueW(
            (-2147483647_isize) as Handle,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide("SystemUsesLightTheme").as_ptr(),
            0x10,
            null_mut(),
            (&mut light as *mut u32).cast(),
            &mut length,
        ) == 0
            && light <= 1
        {
            dark = light == 0;
        }
        (dark, contrast.flags & 1 != 0, rgb)
    }
}
pub fn system_language() -> String {
    let mut value = [0u16; 85];
    // SAFETY: writable buffer holds LOCALE_NAME_MAX_LENGTH UTF-16 code units.
    let count = unsafe { GetUserDefaultLocaleName(value.as_mut_ptr(), value.len() as i32) };
    if count > 0 {
        String::from_utf16_lossy(&value[..count as usize - 1])
    } else {
        "en".into()
    }
}
pub fn copy_text(text: &str) -> io::Result<()> {
    let text = wide(text);
    // SAFETY: clipboard allocation remains ours until successful SetClipboardData,
    // which transfers ownership to the OS. Clipboard is closed on every path.
    unsafe {
        if OpenClipboard(null_mut()) == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            let memory = GlobalAlloc(2, text.len() * 2);
            if memory.is_null() {
                return Err(io::Error::last_os_error());
            }
            let pointer = GlobalLock(memory);
            if pointer.is_null() {
                GlobalFree(memory);
                return Err(io::Error::last_os_error());
            }
            std::ptr::copy_nonoverlapping(text.as_ptr(), pointer.cast(), text.len());
            GlobalUnlock(memory);
            if EmptyClipboard() == 0 || SetClipboardData(13, memory).is_null() {
                GlobalFree(memory);
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })();
        CloseClipboard();
        result
    }
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

/// One sleeping native wait owns a change handle and a cancel event. It does not
/// poll a file or wake the UI without a directory notification.
pub struct Watcher {
    stop: usize,
    thread: Option<JoinHandle<()>>,
    directory: std::path::PathBuf,
}
impl Watcher {
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    fn new(path: &Path, proxy: Proxy) -> io::Result<Self> {
        use std::os::windows::ffi::OsStrExt;
        let mut directory = path
            .parent()
            .ok_or_else(|| io::Error::other("config parent missing"))?;
        while !directory.exists() {
            directory = directory
                .parent()
                .ok_or_else(|| io::Error::other("config ancestor missing"))?;
        }
        let directory = directory.to_path_buf();
        let path: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: paths live through calls; both returned handles are owned. The
        // thread gets integer handle values, accesses no Rust pointer and exits
        // before Drop closes the cancellation event.
        let change = unsafe { FindFirstChangeNotificationW(path.as_ptr(), 0, 0x1 | 0x10) };
        if change as isize == -1 {
            return Err(io::Error::last_os_error());
        }
        let stop = unsafe { CreateEventW(null(), 1, 0, null()) };
        if stop.is_null() {
            unsafe {
                FindCloseChangeNotification(change);
            }
            return Err(io::Error::last_os_error());
        }
        let change_value = change as usize;
        let stop_value = stop as usize;
        let thread = std::thread::Builder::new()
            .name("systeminfo-config".into())
            .stack_size(65536)
            .spawn(move || {
                let handles = [stop_value as Handle, change_value as Handle];
                loop {
                    let result =
                        unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, u32::MAX) };
                    if result != 1 {
                        break;
                    }
                    proxy.post(CONFIG_CHANGED);
                    if unsafe { FindNextChangeNotification(change_value as Handle) } == 0 {
                        break;
                    }
                }
                unsafe {
                    FindCloseChangeNotification(change_value as Handle);
                }
            });
        match thread {
            Ok(thread) => Ok(Self {
                stop: stop_value,
                thread: Some(thread),
                directory,
            }),
            Err(error) => {
                unsafe {
                    CloseHandle(stop);
                    FindCloseChangeNotification(change);
                }
                Err(error)
            }
        }
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            SetEvent(self.stop as Handle);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        unsafe {
            CloseHandle(self.stop as Handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Model {
        Model {
            tooltip: "NDDev Systeminfo · العربية · Қазақша".into(),
            entries: vec![
                Entry::Label("Processor: 12%".into()),
                Entry::Separator,
                Entry::Submenu {
                    title: "Settings".into(),
                    entries: vec![Entry::Action {
                        id: 1,
                        title: "Realtime".into(),
                        enabled: true,
                        checked: true,
                    }],
                },
            ],
            rtl: true,
            icon: vec![255; 32 * 32 * 4],
            icon_side: 32,
        }
    }
    #[test]
    fn native_layouts_match_the_windows_x64_sdk() {
        assert_eq!(size_of::<WindowClass>(), 72);
        assert_eq!(size_of::<NotifyIcon>(), 976);
        assert_eq!(size_of::<Message>(), 48);
        assert_eq!(size_of::<BitmapHeader>(), 40);
        assert_eq!(std::mem::offset_of!(NotifyIcon, icon), 32);
        assert_eq!(std::mem::offset_of!(NotifyIcon, version), 816);
    }
    #[test]
    fn native_icon_menu_watch_and_window_ownership_repeatedly_close() {
        for attempt in 0..3 {
            let identity = format!(
                "org.nddev.systeminfo.test.{}.{}",
                std::process::id(),
                attempt
            );
            let tray = Tray::new(&identity, model()).unwrap().unwrap();
            assert!(Tray::new(&identity, model()).unwrap().is_none());
            assert!(tray.dpi() >= 96);
            let proxy = tray.proxy();
            let directory = std::env::temp_dir().join(&identity);
            std::fs::create_dir(&directory).unwrap();
            let file = directory.join("config.toml");
            let watcher = tray.watch_config(&file).unwrap();
            assert_eq!(watcher.directory(), directory);
            let writer_file = file.clone();
            let writer = std::thread::spawn(move || {
                std::fs::write(writer_file, "[display]\nlocale=\"ar\"\n").unwrap()
            });
            let mut observed = false;
            tray.run(|event, _| {
                if let Event::ConfigurationChanged = event
                    && std::fs::read_to_string(&file).is_ok_and(|text| text.contains("ar"))
                {
                    observed = true;
                    proxy.close();
                }
            })
            .unwrap();
            assert!(observed);
            drop(watcher);
            writer.join().unwrap();
            // A stale proxy is inert after native Drop; it cannot address a new
            // window which later reuses the same numeric handle.
            proxy.update(model());
            proxy.close();
            std::fs::remove_dir_all(directory).unwrap();
        }
        assert!(!system_language().is_empty());
        let _ = system_appearance();
        let menu = create_menu(&model().entries).unwrap();
        drop(menu);
        let mut invalid = model();
        invalid.icon_side = 1;
        assert!(create_icon(&invalid).is_err());
    }
}

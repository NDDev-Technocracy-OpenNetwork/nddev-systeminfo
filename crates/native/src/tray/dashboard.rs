//! Creator-thread Win32 analysis window. Standard BUTTON/COMBOBOX controls keep
//! keyboard and accessibility semantics; only painting is application-owned.
use super::{ffi::*, wide};
use std::{
    cell::RefCell,
    io,
    marker::PhantomData,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    rc::Rc,
};

pub const OPEN: u32 = 900;
pub const CLOSED: u32 = 901;
const COMMAND: u32 = 0x8008;
const CLASS: &str = "org.nddev.systeminfo.analysis";
#[derive(Clone, PartialEq)]
pub struct Action {
    pub id: u32,
    pub text: String,
    pub selected: bool,
    pub enabled: bool,
}
#[derive(Clone, PartialEq)]
pub struct Point {
    pub x: f64,
    pub low: f64,
    pub high: f64,
    pub value: f64,
    pub gap: bool,
}
#[derive(Clone, PartialEq)]
pub enum Row {
    Text(String),
    Buttons {
        columns: usize,
        large: bool,
        items: Vec<Action>,
    },
    Choice {
        id: u32,
        items: Vec<String>,
        selected: usize,
    },
    Plot {
        points: Vec<Point>,
        description: String,
    },
}
#[derive(Clone, PartialEq)]
pub struct Model {
    pub title: String,
    pub rtl: bool,
    /// Win32 COLORREF: surface, raised, text, secondary, border, accent, focus.
    pub colors: [u32; 7],
    pub padding: i32,
    pub gap: i32,
    pub control: i32,
    pub font: i32,
    pub card: i32,
    pub radius: i32,
    pub plot_height: i32,
    pub width: i32,
    pub height: i32,
    pub rows: Vec<Row>,
}
struct Control {
    handle: Handle,
    id: u32,
    text: String,
    row: usize,
    index: usize,
}
struct Surface {
    window: Handle,
    parent: Handle,
    model: Model,
    controls: Vec<Control>,
    font: Handle,
    card_font: Handle,
    background: Handle,
    scroll: i32,
    total: i32,
    dpi: f64,
    plots: Vec<(Rectangle, Vec<Point>)>,
}
thread_local! { static SURFACE: RefCell<Option<Rc<RefCell<Surface>>>> = const { RefCell::new(None) }; }
pub(super) struct Window {
    surface: Rc<RefCell<Surface>>,
    instance: Handle,
    icon: Handle,
    _thread: PhantomData<Rc<()>>,
}
impl Window {
    pub(super) fn new(parent: Handle, model: Model, source_icon: Handle) -> io::Result<Self> {
        validate(&model)?;
        // SAFETY: synchronous Win32 copies class/name strings. No Rust pointer
        // enters window data; the TLS map is consulted only on the owning thread.
        unsafe {
            let instance = GetModuleHandleW(null());
            let icon = CopyIcon(source_icon);
            if icon.is_null() {
                return Err(io::Error::last_os_error());
            }
            let class = wide(CLASS);
            let definition = WindowClass {
                style: 3,
                procedure: Some(procedure),
                class_extra: 0,
                window_extra: 0,
                instance,
                icon,
                cursor: LoadCursorW(null_mut(), 32512_usize as *const u16),
                background: null_mut(),
                menu_name: null(),
                class_name: class.as_ptr(),
            };
            if RegisterClassW(&definition) == 0 {
                DestroyIcon(icon);
                return Err(io::Error::last_os_error());
            }
            let mut area: Rectangle = zeroed();
            SystemParametersInfoW(48, 0, (&mut area as *mut Rectangle).cast(), 0);
            let width = model
                .width
                .min((area.right - area.left - 2 * model.padding).max(320));
            let height = model
                .height
                .min((area.bottom - area.top - 2 * model.padding).max(320));
            let window = CreateWindowExW(
                0x00010000,
                class.as_ptr(),
                wide(&model.title).as_ptr(),
                0x00cf0000 | 0x00200000,
                area.left + (area.right - area.left - width).max(0) / 2,
                area.top + (area.bottom - area.top - height).max(0) / 2,
                width,
                height,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            );
            if window.is_null() {
                UnregisterClassW(class.as_ptr(), instance);
                DestroyIcon(icon);
                return Err(io::Error::last_os_error());
            }
            let dpi = f64::from(GetDpiForWindow(window).max(96)) / 96.0;
            let surface = Rc::new(RefCell::new(Surface {
                window,
                parent,
                model,
                controls: Vec::new(),
                font: null_mut(),
                card_font: null_mut(),
                background: null_mut(),
                scroll: 0,
                total: 0,
                dpi,
                plots: Vec::new(),
            }));
            SURFACE.with(|slot| *slot.borrow_mut() = Some(surface.clone()));
            let mut result = Self {
                surface,
                instance,
                icon,
                _thread: PhantomData,
            };
            result.rebuild();
            ShowWindow(window, 5);
            SetForegroundWindow(window);
            UpdateWindow(window);
            Ok(result)
        }
    }
    pub(super) fn update(&mut self, model: Model) -> io::Result<()> {
        validate(&model)?;
        let changed = {
            let mut s = self.surface.borrow_mut();
            if s.model == model {
                return Ok(());
            }
            let shape = |m: &Model| {
                m.rows
                    .iter()
                    .map(|r| match r {
                        Row::Buttons {
                            items,
                            columns,
                            large,
                        } => format!(
                            "b:{columns}:{large}:{:?}",
                            items.iter().map(|a| a.id).collect::<Vec<_>>()
                        ),
                        Row::Choice { id, items, .. } => format!("c:{id}:{items:?}"),
                        Row::Text(_) => "t".into(),
                        Row::Plot { .. } => "p".into(),
                    })
                    .collect::<Vec<_>>()
            };
            let changed = shape(&s.model) != shape(&model)
                || s.model.font != model.font
                || s.model.card != model.card
                || s.model.colors != model.colors
                || s.model.rtl != model.rtl;
            s.model = model;
            changed
        };
        if changed {
            self.rebuild();
        } else {
            self.surface.borrow_mut().layout();
        }
        let s = self.surface.borrow();
        // SAFETY: the creator-thread window remains owned by Self.
        unsafe {
            SetWindowTextW(s.window, wide(&s.model.title).as_ptr());
            InvalidateRect(s.window, null(), 1);
        }
        Ok(())
    }
    fn rebuild(&mut self) {
        self.surface.borrow_mut().rebuild();
    }
    pub(super) fn dialog(&self, message: &Message) -> bool {
        let window = self.surface.borrow().window;
        // SAFETY: MSG was initialized by GetMessage. Standard dialog navigation
        // operates only on this owned window and its standard child controls.
        let handled = unsafe { IsDialogMessageW(window, message) != 0 };
        if handled {
            self.surface.borrow_mut().reveal_focus();
        }
        handled
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        SURFACE.with(|slot| {
            slot.borrow_mut().take();
        });
        let mut s = self.surface.borrow_mut();
        // SAFETY: remove TLS before destruction to prevent reentrant access.
        // DestroyWindow destroys children; selected GDI objects are restored by
        // every paint operation before these owned fonts/brushes are deleted.
        unsafe {
            DestroyWindow(s.window);
            s.controls.clear();
            s.release_gdi();
            DestroyIcon(self.icon);
            UnregisterClassW(wide(CLASS).as_ptr(), self.instance);
        }
    }
}
fn validate(model: &Model) -> io::Result<()> {
    let mut ids = std::collections::BTreeSet::new();
    if model.rows.len() > 128
        || !(1..=64).contains(&model.font)
        || !(1..=96).contains(&model.card)
        || !(0..=64).contains(&model.padding)
        || !(0..=64).contains(&model.gap)
        || !(16..=128).contains(&model.control)
        || !(0..=64).contains(&model.radius)
        || !(1..=1024).contains(&model.plot_height)
        || !(320..=4096).contains(&model.width)
        || !(320..=2160).contains(&model.height)
        || model.rows.iter().any(|r| match r {
            Row::Buttons { columns, items, .. } => {
                *columns == 0
                    || *columns > 16
                    || items.len() > 64
                    || items.iter().any(|a| {
                        a.id == 0 || a.id > 60000 || !ids.insert(a.id) || a.text.len() > 4096
                    })
            }
            Row::Choice {
                id,
                items,
                selected,
            } => {
                !ids.insert(*id)
                    || *id == 0
                    || *id > 60000
                    || items.len() > 512
                    || (!items.is_empty() && *selected >= items.len())
                    || items.iter().any(|s| s.len() > 4096)
            }
            Row::Plot {
                points,
                description,
            } => {
                points.len() > 8192
                    || description.len() > 4096
                    || points.iter().any(|p| {
                        [p.x, p.low, p.high, p.value]
                            .iter()
                            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                    })
            }
            Row::Text(s) => s.len() > 16384,
        })
    {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "native dashboard model exceeds bounds",
        ))
    } else {
        Ok(())
    }
}
impl Surface {
    fn px(&self, value: i32) -> i32 {
        (f64::from(value) * self.dpi).round() as i32
    }
    unsafe fn release_gdi(&mut self) {
        // SAFETY: these are owned, unselected GDI objects, or null.
        unsafe {
            for object in [&mut self.font, &mut self.card_font, &mut self.background] {
                if !object.is_null() {
                    DeleteObject(*object);
                    *object = null_mut();
                }
            }
        }
    }
    fn rebuild(&mut self) {
        // SAFETY: all calls are synchronous on the creator thread; native controls
        // copy their text. Old handles are removed before replacements are stored.
        unsafe {
            for c in self.controls.drain(..) {
                DestroyWindow(c.handle);
            }
            self.release_gdi();
            self.font = font(self.px(self.model.font));
            self.card_font = font(self.px(self.model.card));
            self.background = CreateSolidBrush(self.model.colors[0]);
            let instance = GetModuleHandleW(null());
            for (row, value) in self.model.rows.iter().enumerate() {
                let items: Vec<(u32, String, bool)> = match value {
                    Row::Buttons { items, .. } => items
                        .iter()
                        .map(|a| (a.id, a.text.clone(), false))
                        .collect(),
                    Row::Choice { id, .. } => vec![(*id, String::new(), true)],
                    _ => Vec::new(),
                };
                for (index, (id, text, choice)) in items.into_iter().enumerate() {
                    let class = wide(if choice { "COMBOBOX" } else { "BUTTON" });
                    let handle = CreateWindowExW(
                        if self.model.rtl { 0x00400000 } else { 0 },
                        class.as_ptr(),
                        wide(&text).as_ptr(),
                        0x50010000
                            | if choice {
                                0x00200000 | 3 | 0x10 | 0x200
                            } else {
                                0x0000000b
                            },
                        0,
                        0,
                        1,
                        1,
                        self.window,
                        id as usize as Handle,
                        instance,
                        null_mut(),
                    );
                    if handle.is_null() {
                        continue;
                    }
                    SendMessageW(handle, 0x0030, self.font as usize, 1);
                    if let Row::Choice {
                        items, selected, ..
                    } = value
                    {
                        for item in items {
                            SendMessageW(handle, 0x0143, 0, wide(item).as_ptr() as isize);
                        }
                        SendMessageW(handle, 0x014e, *selected, 0);
                        SendMessageW(
                            handle,
                            0x0153,
                            usize::MAX,
                            self.px(self.model.control) as isize,
                        );
                        SendMessageW(
                            handle,
                            0x0153,
                            0,
                            self.px(self.model.font + self.model.gap) as isize,
                        );
                    }
                    self.controls.push(Control {
                        handle,
                        id,
                        text,
                        row,
                        index,
                    });
                }
            }
            self.layout();
            InvalidateRect(self.window, null(), 1);
        }
    }
    fn row_height(&self, row: &Row) -> i32 {
        match row {
            Row::Text(_) => self.px(self.model.font + self.model.gap),
            Row::Buttons {
                columns,
                large,
                items,
            } => {
                (items.len().div_ceil(*columns) as i32)
                    * (self.px(if *large {
                        self.model.font + self.model.card + 2 * self.model.padding
                    } else {
                        self.model.control
                    }) + self.px(self.model.gap))
            }
            Row::Choice { .. } => self.px(self.model.control + self.model.gap),
            Row::Plot { .. } => self.px(self.model.plot_height),
        }
    }
    fn layout(&mut self) {
        // SAFETY: rectangles and scroll state are initialized before Win32 reads;
        // every child handle belongs to this Surface until its next rebuild.
        unsafe {
            let mut rect: Rectangle = zeroed();
            GetClientRect(self.window, &mut rect);
            let padding = self.px(self.model.padding);
            let gap = self.px(self.model.gap);
            let width = (rect.right - 2 * padding).max(self.px(160));
            let choice_height = self.px(self.model.control * 12);
            self.total = self
                .model
                .rows
                .iter()
                .map(|r| self.row_height(r) + gap)
                .sum::<i32>()
                + 2 * padding;
            self.scroll = self.scroll.clamp(0, (self.total - rect.bottom).max(0));
            let info = ScrollInfo {
                size: size_of::<ScrollInfo>() as u32,
                mask: 1 | 2 | 4,
                min: 0,
                max: self.total.saturating_sub(1),
                page: rect.bottom.max(0) as u32,
                position: self.scroll,
                track: 0,
            };
            SetScrollInfo(self.window, 1, &info, 1);
            self.plots.clear();
            let mut y = padding - self.scroll;
            for (row, value) in self.model.rows.iter().enumerate() {
                let height = self.row_height(value);
                for control in self.controls.iter_mut().filter(|c| c.row == row) {
                    match value {
                        Row::Buttons {
                            columns,
                            items,
                            large,
                        } => {
                            let cell_width =
                                (width - gap * (*columns as i32 - 1)) / *columns as i32;
                            let cell_height = (if *large {
                                self.model.font + self.model.card + 2 * self.model.padding
                            } else {
                                self.model.control
                            }) as f64
                                * self.dpi;
                            let column = control.index % columns;
                            let column = if self.model.rtl {
                                columns - 1 - column
                            } else {
                                column
                            };
                            MoveWindow(
                                control.handle,
                                padding + column as i32 * (cell_width + gap),
                                y + (control.index / columns) as i32 * (cell_height as i32 + gap),
                                cell_width,
                                cell_height as i32,
                                1,
                            );
                            let action = &items[control.index];
                            if control.text != action.text {
                                SetWindowTextW(control.handle, wide(&action.text).as_ptr());
                                control.text.clone_from(&action.text);
                            }
                            EnableWindow(control.handle, i32::from(action.enabled));
                            InvalidateRect(control.handle, null(), 0);
                        }
                        Row::Choice { selected, .. } => {
                            MoveWindow(control.handle, padding, y, width, choice_height, 1);
                            SendMessageW(control.handle, 0x014e, *selected, 0);
                        }
                        _ => {}
                    }
                }
                if let Row::Plot { points, .. } = value {
                    self.plots.push((
                        Rectangle {
                            left: padding,
                            top: y,
                            right: padding + width,
                            bottom: y + height,
                        },
                        points.clone(),
                    ));
                }
                y += height + gap;
            }
        }
    }
    fn reveal_focus(&mut self) {
        // SAFETY: focus is borrowed and must be one of our controls before use.
        unsafe {
            let focus = GetFocus();
            if !self.controls.iter().any(|c| c.handle == focus) {
                return;
            }
            let mut rect: Rectangle = zeroed();
            let mut client: Rectangle = zeroed();
            GetWindowRect(focus, &mut rect);
            GetClientRect(self.window, &mut client);
            let mut point = super::ffi::Point {
                x: rect.left,
                y: rect.top,
            };
            ScreenToClient(self.window, &mut point);
            if point.y < 0 {
                self.scroll = (self.scroll + point.y).max(0);
            } else if point.y + rect.bottom - rect.top > client.bottom {
                self.scroll += point.y + rect.bottom - rect.top - client.bottom;
            }
            self.layout();
            InvalidateRect(self.window, null(), 1);
        }
    }
    fn paint(&self) {
        // SAFETY: BeginPaint/EndPaint and every selected GDI object are balanced
        // within this call. Drawing never retains a DC or a borrowed UTF-16 pointer.
        unsafe {
            let mut paint: Paint = zeroed();
            let dc = BeginPaint(self.window, &mut paint);
            FillRect(dc, &paint.rectangle, self.background);
            let old = SelectObject(dc, self.font);
            SetBkMode(dc, 1);
            SetTextColor(dc, self.model.colors[2]);
            let padding = self.px(self.model.padding);
            let gap = self.px(self.model.gap);
            let mut client: Rectangle = zeroed();
            GetClientRect(self.window, &mut client);
            let mut y = padding - self.scroll;
            for row in &self.model.rows {
                let height = self.row_height(row);
                if let Row::Text(text) = row {
                    let mut rect = Rectangle {
                        left: padding,
                        top: y,
                        right: client.right - padding,
                        bottom: y + height,
                    };
                    DrawTextW(
                        dc,
                        wide(&gdi_text(text)).as_ptr(),
                        -1,
                        &mut rect,
                        0x10 | 0x800 | 0x8000 | if self.model.rtl { 0x20000 | 2 } else { 0 },
                    );
                }
                y += height + gap;
            }
            for (rect, points) in &self.plots {
                let brush = CreateSolidBrush(self.model.colors[1]);
                FillRect(dc, rect, brush);
                DeleteObject(brush);
                let grid = CreatePen(0, self.px(1), self.model.colors[4]);
                let previous = SelectObject(dc, grid);
                for n in 0..=4 {
                    let y = rect.top + (rect.bottom - rect.top) * n / 4;
                    MoveToEx(dc, rect.left, y, null_mut());
                    LineTo(dc, rect.right, y);
                }
                SelectObject(dc, previous);
                DeleteObject(grid);
                let ink = CreatePen(0, self.px(1).max(1), self.model.colors[5]);
                let previous = SelectObject(dc, ink);
                let mut last = None;
                for p in points {
                    let x = rect.left + (f64::from(rect.right - rect.left) * p.x).round() as i32;
                    let y =
                        rect.bottom - (f64::from(rect.bottom - rect.top) * p.value).round() as i32;
                    if let Some((lx, ly)) = last
                        && !p.gap
                    {
                        MoveToEx(dc, lx, ly, null_mut());
                        LineTo(dc, x, y);
                    }
                    let low =
                        rect.bottom - (f64::from(rect.bottom - rect.top) * p.low).round() as i32;
                    let high =
                        rect.bottom - (f64::from(rect.bottom - rect.top) * p.high).round() as i32;
                    MoveToEx(dc, x, low, null_mut());
                    LineTo(dc, x, high);
                    MoveToEx(dc, x - 1, y, null_mut());
                    LineTo(dc, x + 1, y);
                    last = Some((x, y));
                }
                SelectObject(dc, previous);
                DeleteObject(ink);
            }
            SelectObject(dc, old);
            EndPaint(self.window, &paint);
        }
    }
    fn draw_button(&self, item: &DrawItem) {
        let Some(control) = self.controls.iter().find(|c| c.id == item.id) else {
            return;
        };
        let Row::Buttons { items, large, .. } = &self.model.rows[control.row] else {
            return;
        };
        let action = &items[control.index];
        // SAFETY: WM_DRAWITEM provides the DC/rectangle for this standard child
        // during the callback. All created GDI objects are unselected then deleted.
        unsafe {
            let fill = CreateSolidBrush(self.model.colors[if item.state & 1 != 0 { 0 } else { 1 }]);
            let pen = CreatePen(
                0,
                self.px(if action.selected { 2 } else { 1 }),
                self.model.colors[if action.selected || item.state & 16 != 0 {
                    6
                } else {
                    4
                }],
            );
            let old_brush = SelectObject(item.dc, fill);
            let old_pen = SelectObject(item.dc, pen);
            RoundRect(
                item.dc,
                item.rectangle.left,
                item.rectangle.top,
                item.rectangle.right,
                item.rectangle.bottom,
                self.px(self.model.radius),
                self.px(self.model.radius),
            );
            SetBkMode(item.dc, 1);
            SetTextColor(item.dc, self.model.colors[2]);
            let old_font = SelectObject(item.dc, self.font);
            let mut rect = Rectangle {
                left: item.rectangle.left + self.px(self.model.gap),
                top: item.rectangle.top
                    + self.px(if *large {
                        self.model.gap
                    } else {
                        self.model.gap / 4
                    }),
                right: item.rectangle.right - self.px(self.model.gap),
                bottom: item.rectangle.bottom
                    - self.px(if *large {
                        self.model.gap
                    } else {
                        self.model.gap / 4
                    }),
            };
            if *large && let Some((caption, value)) = action.text.split_once('\n') {
                SetTextColor(item.dc, self.model.colors[3]);
                let mut title = Rectangle {
                    bottom: rect.top + self.px(self.model.font + self.model.gap),
                    ..rect
                };
                DrawTextW(
                    item.dc,
                    wide(caption).as_ptr(),
                    -1,
                    &mut title,
                    0x20 | 0x800 | 0x8000,
                );
                SelectObject(item.dc, self.card_font);
                SetTextColor(item.dc, self.model.colors[2]);
                rect.top = title.bottom;
                DrawTextW(
                    item.dc,
                    wide(value).as_ptr(),
                    -1,
                    &mut rect,
                    0x20 | 0x4 | 0x800 | 0x8000,
                );
            } else {
                DrawTextW(
                    item.dc,
                    wide(&gdi_text(&action.text)).as_ptr(),
                    -1,
                    &mut rect,
                    1 | 4 | 0x20 | 0x800 | 0x8000,
                );
            }
            SelectObject(item.dc, old_font);
            SelectObject(item.dc, old_brush);
            SelectObject(item.dc, old_pen);
            DeleteObject(fill);
            DeleteObject(pen);
        }
    }
    fn draw_choice(&self, item: &DrawItem) {
        let Some(control) = self.controls.iter().find(|c| c.id == item.id) else {
            return;
        };
        let Row::Choice {
            items, selected, ..
        } = &self.model.rows[control.row]
        else {
            return;
        };
        let index = if item.item == u32::MAX {
            *selected
        } else {
            item.item as usize
        };
        let Some(text) = items.get(index) else {
            return;
        };
        // SAFETY: standard COMBOBOX keeps its strings/accessibility/selection;
        // the callback lends only this item's initialized DC and rectangle.
        unsafe {
            let brush =
                CreateSolidBrush(self.model.colors[if item.state & 1 != 0 { 0 } else { 1 }]);
            FillRect(item.dc, &item.rectangle, brush);
            DeleteObject(brush);
            let old = SelectObject(item.dc, self.font);
            SetBkMode(item.dc, 1);
            SetTextColor(item.dc, self.model.colors[2]);
            let mut rect = Rectangle {
                left: item.rectangle.left + self.px(self.model.gap),
                right: item.rectangle.right - self.px(self.model.gap),
                ..item.rectangle
            };
            DrawTextW(
                item.dc,
                wide(&gdi_text(text)).as_ptr(),
                -1,
                &mut rect,
                4 | 0x20 | 0x800 | 0x8000 | if self.model.rtl { 0x20000 | 2 } else { 0 },
            );
            if item.state & 16 != 0 {
                DrawFocusRect(item.dc, &item.rectangle);
            }
            SelectObject(item.dc, old);
        }
    }
}
unsafe extern "system" fn procedure(
    window: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    let surface = SURFACE.with(|slot| slot.borrow().clone());
    let Some(surface) = surface else {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    };
    let Ok(mut s) = surface.try_borrow_mut() else {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    };
    // SAFETY: messages are dispatched on this owning thread. Only messages whose
    // SDK contract supplies an initialized structure are dereferenced below.
    unsafe {
        match message {
            0x000f => {
                s.paint();
                0
            }
            0x0014 => 1,
            0x0005 => {
                s.layout();
                InvalidateRect(window, null(), 1);
                0
            }
            0x002b => {
                if lparam != 0 {
                    let item = &*(lparam as *const DrawItem);
                    if item.kind == 3 {
                        s.draw_choice(item);
                    } else {
                        s.draw_button(item);
                    }
                }
                1
            }
            0x0111 => {
                let id = (wparam & 0xffff) as u32;
                let notification = (wparam >> 16) as u32;
                let choice = s
                    .controls
                    .iter()
                    .any(|c| c.id == id && matches!(s.model.rows[c.row], Row::Choice { .. }));
                if choice && notification != 1 {
                    return 0;
                }
                let id = if notification == 1
                    && s.controls
                        .iter()
                        .any(|c| c.id == id && matches!(s.model.rows[c.row], Row::Choice { .. }))
                {
                    let index = SendMessageW(lparam as Handle, 0x0147, 0, 0);
                    if !(0..512).contains(&index) {
                        return 0;
                    }
                    id + index as u32
                } else {
                    id
                };
                PostMessageW(s.parent, COMMAND, id as usize, 0);
                0
            }
            0x0010 => {
                ShowWindow(window, 0);
                PostMessageW(s.parent, COMMAND, CLOSED as usize, 0);
                0
            }
            0x0115 | 0x020a => {
                let mut client: Rectangle = zeroed();
                GetClientRect(window, &mut client);
                if message == 0x020a {
                    s.scroll -=
                        ((wparam >> 16) as u16 as i16 as i32) / 120 * s.px(s.model.control * 3);
                } else {
                    let code = wparam & 0xffff;
                    s.scroll += match code {
                        0 => -s.px(s.model.control),
                        1 => s.px(s.model.control),
                        2 => -client.bottom,
                        3 => client.bottom,
                        _ => 0,
                    };
                    if code == 4 || code == 5 {
                        let mut info: ScrollInfo = zeroed();
                        info.size = size_of::<ScrollInfo>() as u32;
                        info.mask = 16;
                        GetScrollInfo(window, 1, &mut info);
                        s.scroll = info.track;
                    }
                }
                s.layout();
                InvalidateRect(window, null(), 1);
                0
            }
            0x02e0 => {
                s.dpi = f64::from(((wparam >> 16) & 0xffff).max(96) as u32) / 96.0;
                if lparam != 0 {
                    let rect = &*(lparam as *const Rectangle);
                    MoveWindow(
                        window,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        1,
                    );
                }
                s.rebuild();
                PostMessageW(s.parent, 0x8005, 0, 0);
                0
            }
            0x001a | 0x031a => {
                PostMessageW(s.parent, 0x8005, 0, 0);
                0
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}
// GDI predates Unicode isolate controls. LRM preserves numeric direction without
// rendering a missing-glyph box; native layout still uses the OS RTL flag.
fn gdi_text(text: &str) -> String {
    text.replace(['\u{2068}', '\u{2069}'], "\u{200e}")
}
unsafe fn font(height: i32) -> Handle {
    // SAFETY: the returned font is owned and destroyed by Surface.
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            wide("Segoe UI").as_ptr(),
        )
    }
}
#[repr(C)]
struct Paint {
    dc: Handle,
    erase: i32,
    rectangle: Rectangle,
    restore: i32,
    update: i32,
    reserved: [u8; 32],
}
#[repr(C)]
struct DrawItem {
    kind: u32,
    id: u32,
    item: u32,
    action: u32,
    state: u32,
    window: Handle,
    dc: Handle,
    rectangle: Rectangle,
    data: usize,
}
#[repr(C)]
struct ScrollInfo {
    size: u32,
    mask: u32,
    min: i32,
    max: i32,
    page: u32,
    position: i32,
    track: i32,
}
#[link(name = "user32")]
unsafe extern "system" {
    fn DrawFocusRect(dc: Handle, rectangle: *const Rectangle) -> i32;
    fn CopyIcon(icon: Handle) -> Handle;
    fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
    fn ShowWindow(window: Handle, command: i32) -> i32;
    fn SetWindowTextW(window: Handle, text: *const u16) -> i32;
    fn GetClientRect(window: Handle, rectangle: *mut Rectangle) -> i32;
    fn GetWindowRect(window: Handle, rectangle: *mut Rectangle) -> i32;
    fn MoveWindow(window: Handle, x: i32, y: i32, width: i32, height: i32, repaint: i32) -> i32;
    fn SendMessageW(window: Handle, message: u32, wparam: usize, lparam: isize) -> isize;
    fn EnableWindow(window: Handle, enable: i32) -> i32;
    fn IsDialogMessageW(window: Handle, message: *const Message) -> i32;
    fn GetFocus() -> Handle;
    fn ScreenToClient(window: Handle, point: *mut super::ffi::Point) -> i32;
    fn BeginPaint(window: Handle, paint: *mut Paint) -> Handle;
    fn EndPaint(window: Handle, paint: *const Paint) -> i32;
    fn FillRect(dc: Handle, rectangle: *const Rectangle, brush: Handle) -> i32;
    fn DrawTextW(
        dc: Handle,
        text: *const u16,
        length: i32,
        rectangle: *mut Rectangle,
        format: u32,
    ) -> i32;
    fn SetScrollInfo(window: Handle, bar: i32, info: *const ScrollInfo, redraw: i32) -> i32;
    fn GetScrollInfo(window: Handle, bar: i32, info: *mut ScrollInfo) -> i32;
}
#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateSolidBrush(color: u32) -> Handle;
    fn CreatePen(style: i32, width: i32, color: u32) -> Handle;
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strikeout: u32,
        charset: u32,
        output: u32,
        clip: u32,
        quality: u32,
        pitch: u32,
        face: *const u16,
    ) -> Handle;
    fn SelectObject(dc: Handle, object: Handle) -> Handle;
    fn SetBkMode(dc: Handle, mode: i32) -> i32;
    fn SetTextColor(dc: Handle, color: u32) -> u32;
    fn RoundRect(
        dc: Handle,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        width: i32,
        height: i32,
    ) -> i32;
    fn MoveToEx(dc: Handle, x: i32, y: i32, point: *mut super::ffi::Point) -> i32;
    fn LineTo(dc: Handle, x: i32, y: i32) -> i32;
}

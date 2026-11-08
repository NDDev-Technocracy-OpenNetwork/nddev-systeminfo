//! Native CI capture of this process's visible popup, never another app's UI.
use super::{ffi::*, popup_window};
use std::{io, mem::size_of, path::Path, ptr::null_mut};

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

/// Deliver End/Enter only to this process's native popup, never to another
/// application's foreground window. The caller must observe the real command.
pub fn choose_last_action() -> io::Result<()> {
    let window = popup_window(None).ok_or_else(|| io::Error::other("owned popup missing"))?;
    // SAFETY: integer keyboard messages target the owned borrowed popup HWND;
    // no global input injection, pointers or foreign application are involved.
    for key in [0x23usize, 0x0d] {
        if unsafe { PostMessageW(window, 0x100, key, 1) } == 0
            || unsafe { PostMessageW(window, 0x101, key, 0xc0000001) } == 0
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

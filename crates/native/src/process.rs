//! Launch the owned Windows products with an explicit two-handle allowlist.
//! Inheriting every handle can keep a caller's captured stdout alive forever.
use std::{
    ffi::c_void,
    fs::File,
    io,
    mem::{size_of, size_of_val, zeroed},
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::Path,
    ptr::{null, null_mut},
};
type Handle = *mut c_void;
#[repr(C)]
struct Startup {
    size: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    columns: u32,
    rows: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved_size: u16,
    reserved_bytes: *mut u8,
    input: Handle,
    output: Handle,
    error: Handle,
}
#[repr(C)]
struct ExtendedStartup {
    startup: Startup,
    attributes: *mut c_void,
}
#[repr(C)]
struct ProcessInfo {
    process: Handle,
    thread: Handle,
    pid: u32,
    tid: u32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn DuplicateHandle(
        source_process: Handle,
        source: Handle,
        target_process: Handle,
        target: *mut Handle,
        access: u32,
        inherit: i32,
        options: u32,
    ) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn SetFileInformationByHandle(
        file: Handle,
        class: i32,
        information: *const c_void,
        size: u32,
    ) -> i32;
    fn InitializeProcThreadAttributeList(
        list: *mut c_void,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *mut c_void,
        size: usize,
        previous: *mut c_void,
        returned: *mut usize,
    ) -> i32;
    fn CreateProcessW(
        application: *const u16,
        command: *mut u16,
        process_security: *const c_void,
        thread_security: *const c_void,
        inherit: i32,
        flags: u32,
        environment: *const c_void,
        directory: *const u16,
        startup: *const Startup,
        process: *mut ProcessInfo,
    ) -> i32;
}
struct Owned(Handle);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Attributes(Vec<usize>);
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast());
        }
    }
}
fn duplicate(file: &File) -> io::Result<Owned> {
    let mut handle = null_mut();
    // SAFETY: both process pseudo-handles are borrowed; file stays alive through
    // the call. The new inheritable duplicate is owned and closed exactly once.
    unsafe {
        let process = GetCurrentProcess();
        if DuplicateHandle(process, file.as_raw_handle(), process, &mut handle, 0, 1, 2) == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(Owned(handle))
}
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut result: Vec<_> = path.as_os_str().encode_wide().collect();
    if result.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    result.push(0);
    Ok(result)
}
pub fn spawn_background(executable: &Path, directory: &Path, log: &File) -> io::Result<()> {
    if !executable.is_absolute() || !directory.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "background paths must be absolute",
        ));
    }
    let application = wide(executable)?;
    let directory = wide(directory)?;
    let mut command = vec![u16::from(b'"')];
    command.extend_from_slice(&application[..application.len() - 1]);
    command.extend_from_slice(&[u16::from(b'"'), 0]);
    let input = duplicate(&File::open("NUL")?)?;
    let output = duplicate(log)?;
    let mut handles = [input.0, output.0];
    // SAFETY: the sizing call writes only size. usize storage supplies pointer
    // alignment; initialize succeeds before its RAII destructor is installed.
    // The attribute list and handle array outlive CreateProcessW, which copies
    // the two allowlisted handles into the child. No Rust state is passed.
    unsafe {
        let mut bytes = 0;
        InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes);
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        if InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 1, 0, &mut bytes) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut attributes = Attributes(storage);
        let list = attributes.0.as_mut_ptr().cast();
        if UpdateProcThreadAttribute(
            list,
            0,
            0x00020002,
            handles.as_mut_ptr().cast(),
            size_of_val(&handles),
            null_mut(),
            null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut startup: ExtendedStartup = zeroed();
        startup.startup.size = size_of::<ExtendedStartup>() as u32;
        startup.startup.flags = 0x100; // STARTF_USESTDHANDLES
        startup.startup.input = input.0;
        startup.startup.output = output.0;
        startup.startup.error = output.0;
        startup.attributes = list;
        let mut process: ProcessInfo = zeroed();
        if CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            1,
            0x00080000 | 0x08000000,
            null(),
            directory.as_ptr(),
            &startup.startup,
            &mut process,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        // Process/thread handles are ours; the child owns its inherited copies
        // of stdin/log. Closing these references does not terminate the child.
        let (_process, _thread) = (Owned(process.process), Owned(process.thread));
    }
    Ok(())
}

/// Unlink only this process's executable using Windows' documented POSIX delete
/// semantics. Its mapped image remains valid until process exit; no delayed
/// script, reboot, temporary executable or replacement process is needed.
pub fn unlink_current_executable() -> io::Result<()> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let path = std::env::current_exe()?;
    let file = std::fs::OpenOptions::new()
        .access_mode(0x00010000 | 0x80)
        .share_mode(1 | 2 | 4)
        .custom_flags(0x00200000)
        .open(path)?;
    if file.metadata()?.file_attributes() & 0x400 != 0 {
        return Err(io::Error::other("installed executable is redirected"));
    }
    let flags = 1u32 | 2; // DELETE | POSIX_SEMANTICS; keep mapped image valid.
    // SAFETY: this owned file handle has DELETE access and names our own image.
    // FileDispositionInfoEx (21) consumes the four-byte flags synchronously.
    // Closing the handle unlinks the name; the OS owns the live image mapping.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            21,
            (&flags as *const u32).cast(),
            size_of::<u32>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    drop(file);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_layouts_match_the_windows_x64_sdk() {
        assert_eq!(size_of::<Startup>(), 104);
        assert_eq!(size_of::<ExtendedStartup>(), 112);
        assert_eq!(size_of::<ProcessInfo>(), 24);
    }
}

//! Platform socket activation and owner-only standalone sockets.

use std::io;
#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
pub use uds_windows::{UnixListener, UnixStream};

// The published v1 endpoint is an installation identity, independent of branding.
const SOCKET_NAME: &str = "rldyour-sysinfo.sock";

/// Uses the socket the service manager passed in, or binds one under the
/// platform's per-user directory.
///
/// Returns the listener and whether the manager owns it, which decides if the
/// daemon is allowed to exit when idle.
pub fn bind(explicit: Option<&std::path::Path>) -> std::io::Result<(UnixListener, bool)> {
    if explicit.is_none()
        && let Some(listener) = inherited_listener()?
    {
        return Ok((listener, true));
    }
    let path = explicit
        .map(std::path::Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(socket_path)?;
    if !path.is_absolute() {
        return Err(io::Error::other("socket path must be absolute"));
    }

    let listener = bind_at(&path)?;

    // systemd already delivers the socket at 0600; a self-bound one gets the
    // same restriction explicitly rather than whatever the umask happens to be.
    // Only the owner may connect to a socket that reports this host's metrics.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok((listener, false))
}

fn bind_at(path: &std::path::Path) -> io::Result<UnixListener> {
    match UnixListener::bind(path) {
        Ok(listener) => Ok(listener),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileTypeExt;
                if !std::fs::symlink_metadata(path)?.file_type().is_socket() {
                    return Err(error);
                }
            }
            match UnixStream::connect(path) {
                Ok(_) => Err(error), // Never unlink another live daemon.
                Err(probe) if probe.kind() == io::ErrorKind::ConnectionRefused => {
                    std::fs::remove_file(path)?;
                    UnixListener::bind(path)
                }
                Err(probe) => Err(probe),
            }
        }
        Err(error) => Err(error),
    }
}

/// The socket location follows each platform's own convention, and only that
/// convention: honouring `XDG_RUNTIME_DIR` outside Linux would strand the
/// clients, which look in the platform directory and nowhere else.
pub fn socket_path() -> std::io::Result<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
            std::io::Error::other("XDG_RUNTIME_DIR is unset and no socket was inherited")
        })?;
        return Ok(std::path::Path::new(&runtime).join(SOCKET_NAME));
    }

    #[cfg(target_os = "macos")]
    {
        let home =
            std::env::var_os("HOME").ok_or_else(|| std::io::Error::other("HOME is unset"))?;
        // Library/Caches is purgeable under disk pressure, which would strand
        // the socket of a live daemon; Application Support is not.
        let directory =
            std::path::Path::new(&home).join("Library/Application Support/rldyour-sysinfo");
        std::fs::create_dir_all(&directory)?;
        return Ok(directory.join(SOCKET_NAME));
    }

    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("TEMP"))
            .ok_or_else(|| std::io::Error::other("LOCALAPPDATA and TEMP are unset"))?;
        let directory = std::path::Path::new(&base).join("rldyour-sysinfo");
        std::fs::create_dir_all(&directory)?;
        return Ok(directory.join(SOCKET_NAME));
    }

    #[allow(unreachable_code)]
    Err(std::io::Error::other("no socket location on this platform"))
}

/// The listening socket the platform service manager passed in, if any.
///
/// systemd hands its socket over as descriptor 3 when LISTEN_FDS/LISTEN_PID
/// name this process; launchd keeps it in the `Sockets` dictionary and hands
/// it out through `launch_activate_socket`. Both let the daemon exit when
/// idle because the next connection relaunches it.
#[cfg(target_os = "linux")]
fn inherited_listener() -> io::Result<Option<UnixListener>> {
    let listening = std::env::var("LISTEN_FDS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok());
    let owner = std::env::var("LISTEN_PID")
        .ok()
        .and_then(|v| v.parse::<u32>().ok());
    if owner != Some(std::process::id()) || listening == Some(0) || listening.is_none() {
        return Ok(None);
    }
    if listening != Some(1) {
        return Err(io::Error::other("expected exactly one activation socket"));
    }
    use std::os::fd::FromRawFd;
    // SAFETY: systemd guarantees descriptor 3 is the listening socket it
    // created for this unit, and it is passed to exactly one process.
    let listener = unsafe { UnixListener::from_raw_fd(3) };
    prepare_inherited(&listener)?;
    Ok(Some(listener))
}

#[cfg(target_os = "macos")]
fn inherited_listener() -> io::Result<Option<UnixListener>> {
    use std::os::fd::FromRawFd;
    use std::os::raw::{c_char, c_int};

    unsafe extern "C" {
        /// In libSystem since launchd exists; returns an error code when this
        /// process was not launched with a `Sockets` dictionary.
        fn launch_activate_socket(
            name: *const c_char,
            fds: *mut *mut c_int,
            cnt: *mut usize,
        ) -> c_int;
    }

    // The key must match the plist's `Sockets` entry.
    let mut fds: *mut c_int = std::ptr::null_mut();
    let mut count: usize = 0;
    if unsafe { launch_activate_socket(c"sock".as_ptr(), &mut fds, &mut count) } != 0
        || fds.is_null()
        || count == 0
    {
        return Ok(None);
    }
    // The plist declares exactly one socket; the array is launchd-owned and
    // the caller frees it.
    let fd = unsafe { *fds };
    for index in 1..count {
        unsafe { libc::close(*fds.add(index)) };
    }
    unsafe { libc::free(fds.cast()) };
    // SAFETY: launch_activate_socket returned this descriptor to us; it is a
    // listening socket nobody else owns.
    let listener = unsafe { UnixListener::from_raw_fd(fd) };
    if count != 1 {
        return Err(io::Error::other("expected exactly one launchd socket"));
    }
    prepare_inherited(&listener)?;
    Ok(Some(listener))
}

#[cfg(target_os = "windows")]
fn inherited_listener() -> io::Result<Option<UnixListener>> {
    // Windows has no per-user service manager with socket activation; the
    // daemon always binds its own and stays resident.
    Ok(None)
}

#[cfg(unix)]
fn prepare_inherited(listener: &UnixListener) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = listener.as_raw_fd();
    #[cfg(target_os = "linux")]
    {
        let mut accepting: libc::c_int = 0;
        let mut length = std::mem::size_of_val(&accepting) as libc::socklen_t;
        // SAFETY: a live owned descriptor and correctly sized output pointers.
        let result = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ACCEPTCONN,
                (&mut accepting as *mut libc::c_int).cast(),
                &mut length,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        if accepting == 0 {
            return Err(io::Error::other("activation descriptor is not a listener"));
        }
    }
    // Darwin does not implement SO_ACCEPTCONN for AF_UNIX. launchd provides
    // the listener contract there; local_addr still rejects non-socket FDs.
    listener.local_addr()?;
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    listener.set_nonblocking(false)?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn path() -> std::path::PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "sysinfo-{}-{}.sock",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn a_second_daemon_cannot_unlink_the_live_listener() {
        let path = path();
        let original = bind_at(&path).unwrap();
        assert_eq!(bind_at(&path).unwrap_err().kind(), io::ErrorKind::AddrInUse);
        let _peer = UnixStream::connect(&path).unwrap();
        drop(original);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn only_a_dead_socket_is_replaced() {
        let path = path();
        drop(bind_at(&path).unwrap());
        let replacement = bind_at(&path).unwrap();
        prepare_inherited(&replacement).unwrap();
        drop(replacement);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_regular_file_is_never_removed() {
        let path = path();
        std::fs::write(&path, b"keep").unwrap();
        assert!(bind_at(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        std::fs::remove_file(path).unwrap();
    }
}

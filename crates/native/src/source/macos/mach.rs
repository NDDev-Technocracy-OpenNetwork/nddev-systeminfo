//! An owned host send right, acquired once and released once. Calling
//! mach_host_self for every sample would accumulate user references.

pub(super) struct Host(pub libc::mach_port_t);

unsafe extern "C" {
    fn mach_host_self() -> libc::mach_port_t;
    static mach_task_self_: libc::mach_port_t;
    fn mach_port_deallocate(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
}

/// The process-owned task port is borrowed, never acquired or released here.
pub(super) fn task_port() -> libc::mach_port_t {
    // SAFETY: libSystem initializes this process-global port before Rust starts.
    unsafe { mach_task_self_ }
}

impl Host {
    pub(super) fn new() -> Self {
        Self(unsafe { mach_host_self() })
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        unsafe { mach_port_deallocate(task_port(), self.0) };
    }
}

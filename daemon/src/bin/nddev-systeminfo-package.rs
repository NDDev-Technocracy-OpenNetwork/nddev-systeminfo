//! dpkg invokes the same small native executable as postinst/prerm/postrm.
#![forbid(unsafe_code)]
fn main() -> std::process::ExitCode {
    #[cfg(target_os = "linux")]
    if let Err(error) = nddev_systeminfo::installation::debian::maintain() {
        nddev_systeminfo::telemetry::failure(&error.to_string());
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

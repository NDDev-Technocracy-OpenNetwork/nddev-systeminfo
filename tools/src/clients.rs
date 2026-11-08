//! Exercise the shipped Python SDK against an owned real native provider.
use crate::Result;
use nddev_systeminfo::{client::Client, config::Config};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Provider(Child, PathBuf);
impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let _ = fs::remove_dir_all(&self.1);
    }
}
pub(super) fn python() -> Result {
    let binaries = crate::build_binaries(false)?;
    let directory = std::env::temp_dir().join(format!("si-python-{}", std::process::id()));
    fs::create_dir(&directory)?;
    let socket = directory.join("s");
    let config = directory.join("config.toml");
    Config::parse(nddev_systeminfo::config::DEFAULT_TOML)?.store(&config)?;
    let child = Command::new(&binaries["nddev-systeminfod"])
        .arg("--socket")
        .arg(&socket)
        .arg("--config")
        .arg(&config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut provider = Provider(child, directory);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(mut client) = Client::connect(&socket, 0, Duration::from_secs(1)) {
            client.next_sample()?;
            break;
        }
        if provider.0.try_wait()?.is_some() || Instant::now() >= deadline {
            return Err("Python acceptance provider did not become ready".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = Command::new("python")
        .args(["python/tests/live.py"])
        .arg(socket)
        .env("PYTHONPATH", fs::canonicalize("python/src")?)
        .status()?;
    if !status.success() {
        return Err("real Python SDK acceptance failed".into());
    }
    Ok(())
}

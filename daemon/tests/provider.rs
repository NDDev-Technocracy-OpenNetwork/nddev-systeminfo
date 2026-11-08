//! Real native daemon acceptance on Linux, macOS and Windows.
use nddev_systeminfo::{
    client::Client,
    config::{Config, DEFAULT_TOML},
};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

struct Provider {
    child: Child,
    directory: PathBuf,
    socket: PathBuf,
    logs: Receiver<serde_json::Value>,
}
impl Provider {
    fn start() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "si-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory).unwrap();
        let socket = directory.join("s");
        let configuration = directory.join("config.toml");
        let mut config = Config::parse(DEFAULT_TOML).unwrap();
        config.sampling.interval_seconds = 0;
        config.sampling.idle_release_seconds = 1;
        config.transport.max_clients = 2;
        config.transport.handshake_timeout_ms = 50;
        config.logging.level = "debug".into();
        config.store(&configuration).unwrap();
        let mut child = Command::new(
            std::env::var_os("CARGO_BIN_EXE_nddev-systeminfod")
                .expect("run just test to provide the native build receipt"),
        )
        .args([
            "--socket",
            socket.to_str().unwrap(),
            "--config",
            configuration.to_str().unwrap(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, logs) = mpsc::sync_channel(32);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str(&line) else {
                    continue;
                };
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        let mut provider = Self {
            child,
            directory,
            socket,
            logs,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !provider.socket.exists() {
            assert!(
                provider.child.try_wait().unwrap().is_none(),
                "native daemon exited before readiness"
            );
            assert!(
                Instant::now() < deadline,
                "native socket readiness deadline exceeded"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        provider
    }
    fn client(&self) -> Client {
        Client::connect(&self.socket, 0, Duration::from_secs(5)).unwrap()
    }
    fn event(&self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let value = self
                .logs
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("lifecycle event deadline exceeded");
            if value["event.name"] == name {
                assert!(value["timestamp"].as_str().is_some());
                assert!(value["severity"].as_str().is_some());
                assert_eq!(value["service.name"], "nddev-systeminfo");
                return;
            }
        }
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn native_provider_is_standalone_lazy_and_releases_its_collector() {
    let mut provider = Provider::start();
    provider.event("systeminfo.started");
    assert!(
        provider.logs.try_recv().is_err(),
        "collector ran without clients"
    );
    let mut client = provider.client();
    let first = client.next_sample().unwrap();
    assert_eq!(first.v, 1);
    assert!(first.memory.used.is_some());
    provider.event("systeminfo.collector.started");
    let second = client.next_sample().unwrap();
    assert!(second.cpu.usage.is_some());
    second.validate().unwrap();
    drop(client);
    provider.event("systeminfo.collector.released");
    assert!(
        provider.child.try_wait().unwrap().is_none(),
        "standalone listener must remain available after idle"
    );
    let mut restarted = provider.client();
    restarted.next_sample().unwrap();
    provider.event("systeminfo.collector.started");
}

#[test]
fn native_provider_admits_only_the_configured_number_of_clients() {
    let provider = Provider::start();
    let mut first = provider.client();
    first.next_sample().unwrap();
    let mut second = provider.client();
    second.next_sample().unwrap();
    let mut excess = provider.client();
    assert!(excess.next_sample().is_err());
    first.next_sample().unwrap();
    second.next_sample().unwrap();
}

#[test]
fn cli_config_and_live_sample_use_the_same_contract() {
    let provider = Provider::start();
    let output = Command::new(
        std::env::var_os("CARGO_BIN_EXE_nddev-systeminfo")
            .expect("run just test to provide the CLI build receipt"),
    )
    .args([
        "sample",
        "--once",
        "--socket",
        provider.socket.to_str().unwrap(),
    ])
    .output()
    .unwrap();
    assert!(output.status.success());
    nddev_systeminfo::protocol::decode(&output.stdout).unwrap();
    let output = Command::new(
        std::env::var_os("CARGO_BIN_EXE_nddev-systeminfo")
            .expect("run just test to provide the CLI build receipt"),
    )
    .args([
        "config",
        "check",
        "--config",
        provider.directory.join("config.toml").to_str().unwrap(),
    ])
    .output()
    .unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["valid"],
        true
    );
}

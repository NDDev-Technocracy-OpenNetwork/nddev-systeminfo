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
        Self::with_clients(2)
    }
    fn with_clients(clients: usize) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "si-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        builder.create(&directory).unwrap();
        let socket = directory.join("s");
        let configuration = directory.join("config.toml");
        let mut config = Config::parse(DEFAULT_TOML).unwrap();
        config.sampling.interval_seconds = 0;
        config.sampling.idle_release_seconds = 1;
        config.transport.max_clients = clients;
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
        let provider = Self {
            child,
            directory,
            socket,
            logs,
        };
        // A Unix socket pathname can exist between bind and listen. The public
        // started event is emitted only after the listener is actually ready.
        provider.event("systeminfo.started");
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
fn native_realtime_cadence_survives_connection_churn_and_invalid_handshakes() {
    use nddev_systeminfo_native::transport::UnixStream;
    use std::io::{Read, Write};
    let provider = Provider::with_clients(32);
    let mut healthy = provider.client();
    healthy.next_sample().unwrap();
    assert!(healthy.next_sample().unwrap().cpu.usage.is_some());
    let mut peers = Vec::new();
    for _ in 0..20 {
        let mut peer = provider.client();
        peer.next_sample().unwrap();
        peers.push(peer);
    }
    let mut durations = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        healthy.next_sample().unwrap();
        durations.push(start.elapsed());
    }
    durations.sort_unstable();
    assert!(
        (Duration::from_millis(400)..Duration::from_millis(1500)).contains(&durations[2]),
        "cadence changed after churn: {durations:?}"
    );
    drop(peers);
    let mut oversized = UnixStream::connect(&provider.socket).unwrap();
    oversized
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    oversized.write_all(&[b'x'; 256]).unwrap();
    let mut byte = [0];
    match oversized.read(&mut byte) {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) => {}
        result => panic!("oversized handshake was not disconnected: {result:?}"),
    }
    let silent = UnixStream::connect(&provider.socket).unwrap();
    silent
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut frame = Vec::new();
    let start = Instant::now();
    BufReader::new(silent)
        .take(4097)
        .read_until(b'\n', &mut frame)
        .unwrap();
    nddev_systeminfo::protocol::decode(&frame).unwrap();
    assert!(start.elapsed() < Duration::from_millis(1500));
    healthy.next_sample().unwrap();
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

#[test]
fn cli_migration_preserves_explicit_legacy_values_and_existing_configuration() {
    let provider = Provider::start();
    let path = provider.directory.join("migrated.toml");
    let executable = std::env::var_os("CARGO_BIN_EXE_nddev-systeminfo").unwrap();
    let migrate = |json: &str| {
        Command::new(&executable)
            .args([
                "config",
                "migrate",
                "--legacy-json",
                json,
                "--config",
                path.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };
    let output = migrate(r#"{"interval":0,"show-network":false,"show-gpu":true}"#);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["migrated"],
        true
    );
    let contents = fs::read_to_string(&path).unwrap();
    let config = Config::parse(&contents).unwrap();
    assert_eq!(config.sampling.interval_seconds, 0);
    assert!(!config.display.show_network);
    assert!(config.display.show_gpu);
    let output = migrate(r#"{"interval":10,"show-network":true}"#);
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["migrated"],
        false
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), contents);
    assert!(!fs::read_dir(&provider.directory).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
    provider.client().next_sample().unwrap();
}

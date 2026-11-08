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
        config.sampling.gpu = false;
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

#[test]
fn legacy_clients_do_not_silently_enable_history_retention() {
    let provider = Provider::with_clients(3);
    let mut ordinary = provider.client();
    for _ in 0..4 {
        ordinary.next_sample().unwrap();
    }
    let mut reader =
        Client::connect_analysis(&provider.socket, 0, Duration::from_secs(5), true).unwrap();
    let mut replayed = 0;
    while reader.next_analysis().unwrap().is_some() {
        replayed += 1;
    }
    assert_eq!(
        replayed, 1,
        "ordinary v1 clients retain only the latest sample"
    );
}

#[test]
fn real_history_replays_once_then_continues_live_with_all_logical_cores() {
    let provider = Provider::with_clients(3);
    let before = fs::read_dir(&provider.directory)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect::<std::collections::BTreeSet<_>>();
    let mut ordinary =
        Client::connect_recording(&provider.socket, 0, Duration::from_secs(5)).unwrap();
    for _ in 0..3 {
        ordinary.next_sample().unwrap();
    }
    let mut analysis =
        Client::connect_analysis(&provider.socket, 0, Duration::from_secs(5), true).unwrap();
    let mut last = 0;
    let mut count = 0;
    while let Some(frame) = analysis.next_analysis().unwrap() {
        assert!(frame.detail.sequence > last);
        last = frame.detail.sequence;
        count += 1;
        assert!(!frame.detail.cores.is_empty());
        assert!(!frame.detail.gpu_enabled);
        assert!(
            frame
                .detail
                .cores
                .iter()
                .all(|core| core.usage.is_none_or(|v| (0.0..=100.0).contains(&v)))
        );
    }
    assert!(count >= 3);
    let live = analysis.next_analysis().unwrap().unwrap();
    assert!(live.detail.sequence > last);
    ordinary.next_sample().unwrap().validate().unwrap();
    let after = fs::read_dir(&provider.directory)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(before, after, "sampling must create no history files");
}

#[test]
fn cli_analysis_returns_a_bounded_view_of_the_real_provider() {
    let provider = Provider::with_clients(3);
    let mut client =
        Client::connect_recording(&provider.socket, 0, Duration::from_secs(5)).unwrap();
    for _ in 0..3 {
        client.next_sample().unwrap();
    }
    let out = provider.directory.join("analysis.json");
    let log = provider.directory.join("analysis.log");
    let mut child = Command::new(std::env::var_os("CARGO_BIN_EXE_nddev-systeminfo").unwrap())
        .args([
            "analyze",
            "--once",
            "--socket",
            provider.socket.to_str().unwrap(),
            "--interval",
            "0",
            "--columns",
            "1",
            "--metric",
            "cpu.usage",
        ])
        .stdout(fs::File::create(&out).unwrap())
        .stderr(fs::File::create(log).unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("analysis command exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
    let data = fs::read(out).unwrap();
    let decoded = nddev_systeminfo::protocol::analysis::decode_view(&data).unwrap();
    assert_eq!(
        decoded.v,
        nddev_systeminfo::protocol::analysis::VIEW_VERSION
    );
    let mut value: serde_json::Value = serde_json::from_slice(&data).unwrap();
    assert_eq!(value["metric"], "cpu.usage");
    assert_eq!(value["unit"], "percent");
    assert_eq!(value["points"].as_array().unwrap().len(), 1);
    assert!(value["statistics"]["samples"].as_u64().unwrap() >= 2);
    assert!(
        !value["latest"]["detail"]["cores"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    value["v"] = 2.into();
    assert!(
        nddev_systeminfo::protocol::analysis::decode_view(&serde_json::to_vec(&value).unwrap())
            .is_err()
    );
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

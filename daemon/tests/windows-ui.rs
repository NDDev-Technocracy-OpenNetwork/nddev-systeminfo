#![cfg(target_os = "windows")]
use nddev_systeminfo::{
    client::Client,
    config::Config,
    presentation::{self as ui, Catalog, Colors, Direction},
};
use nddev_systeminfo_native::tray::{self, Tray};
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Provider(Child, std::path::PathBuf);
impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let _ = fs::remove_dir_all(&self.1);
    }
}

#[test]
fn native_popup_shows_real_provider_readings_in_all_catalogs() {
    let directory = std::env::temp_dir().join(format!("sw-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let socket = directory.join("s");
    let executable = std::env::var_os("CARGO_BIN_EXE_nddev-systeminfod")
        .expect("tools provides the real binary receipt");
    let child = Command::new(executable)
        .arg("--socket")
        .arg(&socket)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _provider = Provider(child, directory);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut client = loop {
        if let Ok(client) = Client::connect(&socket, 0, Duration::from_secs(1)) {
            break client;
        }
        assert!(Instant::now() < deadline, "real provider did not bind");
        thread::sleep(Duration::from_millis(10));
    };
    let artifacts = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".tools/ui-windows");
    fs::create_dir_all(&artifacts).unwrap();
    for (code, theme) in [
        ("en", "light"),
        ("ru", "dark"),
        ("kk", "light-contrast"),
        ("ar", "dark-contrast"),
        ("zh", "light"),
    ] {
        let sample = client.next_sample().unwrap();
        let mut config = Config::parse(nddev_systeminfo::config::DEFAULT_TOML).unwrap();
        config.display.locale = code.into();
        config.display.theme = theme.into();
        let catalog = Catalog::load(code, &[]);
        let colors = Colors::resolve(ui::theme(&config.display, false, false), Direction::On);
        let model = nddev_systeminfo::windows_ui::native_model(&config, Some(&sample));
        assert_eq!(model.rtl, code == "ar");
        let tray = Tray::new(
            &format!(
                "org.nddev.systeminfo.acceptance.{}.{code}",
                std::process::id()
            ),
            model,
        )
        .unwrap()
        .unwrap();
        let proxy = tray.proxy();
        let capture = proxy.clone();
        let path = artifacts.join(format!("{code}-{theme}.png"));
        let photographer = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !tray::acceptance::capture_popup(&path).unwrap() {
                assert!(
                    Instant::now() < deadline,
                    "native popup never became visible"
                );
                thread::sleep(Duration::from_millis(10));
            }
            capture.cancel_menu();
            capture.close();
        });
        proxy.open_menu();
        tray.run(|_, _| {}).unwrap();
        photographer.join().unwrap();
        let text = serde_json::to_string(&sample).unwrap();
        tray::copy_text(&text).unwrap();
        assert_eq!(ui::readings(None).len(), 12);
        assert!(!catalog.text("cpu").is_empty());
        assert!(colors.accent_text.a > 0);
    }
    fs::write(
        artifacts.join("result.json"),
        "{\"native_popup\":true,\"real_provider\":true,\"locales\":5,\"themes\":4}\n",
    )
    .unwrap();
}

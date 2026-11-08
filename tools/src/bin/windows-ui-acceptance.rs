#[cfg(target_os = "windows")]
mod desktop {
    use nddev_systeminfo::{client::Client, config::Config};
    use nddev_systeminfo_native::tray::{self, Tray};
    use std::{
        fs,
        path::Path,
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    struct Provider(Child, std::path::PathBuf);
    struct Close(tray::Proxy);
    impl Drop for Close {
        fn drop(&mut self) {
            self.0.cancel_menu();
            self.0.close();
        }
    }
    impl Drop for Provider {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
            let _ = fs::remove_dir_all(&self.1);
        }
    }

    pub(super) fn run() {
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
            let model = nddev_systeminfo::windows_ui::native_model(&config, Some(&sample));
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
                let _close = Close(capture);
                let deadline = Instant::now() + Duration::from_secs(10);
                while !tray::acceptance::capture_popup(&path).unwrap() {
                    assert!(
                        Instant::now() < deadline,
                        "native popup never became visible"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
            });
            proxy.open_menu();
            tray.run(|_, _| {}).unwrap();
            photographer.join().unwrap();
            let text = serde_json::to_string(&sample).unwrap();
            tray::copy_text(&text).unwrap();
        }
        fs::write(
            artifacts.join("result.json"),
            "{\"native_popup\":true,\"real_provider\":true,\"locales\":5,\"themes\":4}\n",
        )
        .unwrap();
    }
}
fn main() {
    #[cfg(target_os = "windows")]
    desktop::run();
    #[cfg(not(target_os = "windows"))]
    {
        eprintln!("native Windows UI acceptance requires Windows");
        std::process::exit(1);
    }
}

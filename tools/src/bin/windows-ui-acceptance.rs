#[cfg(target_os = "windows")]
mod desktop {
    use nddev_systeminfo::{client::Client, config::Config};
    use nddev_systeminfo_native::tray::{self, Tray};
    use std::{
        fs,
        path::Path,
        process::{Child, Command, Stdio},
        sync::{Arc, Mutex, mpsc},
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
        let provider = Arc::new(Mutex::new(Provider(child, directory)));
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut client = Some(loop {
            if let Ok(client) = Client::connect(&socket, 0, Duration::from_secs(1)) {
                break client;
            }
            assert!(Instant::now() < deadline, "real provider did not bind");
            thread::sleep(Duration::from_millis(10));
        });
        let artifacts = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".tools/ui-windows");
        fs::create_dir_all(&artifacts).unwrap();
        analysis_windows(&socket, &artifacts);
        for (code, theme) in [
            ("en", "light"),
            ("ru", "dark"),
            ("kk", "light-contrast"),
            ("ar", "dark-contrast"),
            ("zh", "light"),
        ] {
            let sample = client.as_mut().unwrap().next_sample().unwrap();
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
            tray::acceptance::focus(&tray).unwrap();
            let proxy = tray.proxy();
            let capture = proxy.clone();
            let path = artifacts.join(format!("{code}-{theme}.png"));
            let offline_path = artifacts.join("offline.png");
            let offline = nddev_systeminfo::windows_ui::native_model(&config, None);
            let mut reader = if code == "zh" { client.take() } else { None };
            let provider = provider.clone();
            let (command, observed) = mpsc::sync_channel(1);
            let photographer = thread::spawn(move || {
                let _close = Close(capture.clone());
                let deadline = Instant::now() + Duration::from_secs(10);
                while !tray::acceptance::capture_popup(&path).unwrap() {
                    assert!(
                        Instant::now() < deadline,
                        "native popup never became visible"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                if let Some(reader) = reader.as_mut() {
                    let mut provider = provider.lock().unwrap();
                    provider.0.kill().unwrap();
                    provider.0.wait().unwrap();
                    while reader.next_sample().is_ok() {
                        assert!(Instant::now() < deadline, "provider EOF was not observed");
                    }
                    capture.update(offline);
                    loop {
                        if let Some(entries) = tray::acceptance::popup_entries().unwrap()
                            && entries
                                .iter()
                                .filter(|(title, _)| title.ends_with('—'))
                                .count()
                                == 12
                            && !entries[15].1
                        // the native copy action is disabled
                        {
                            break;
                        }
                        assert!(
                            Instant::now() < deadline,
                            "open native menu retained disconnected readings"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    assert!(tray::acceptance::capture_popup(&offline_path).unwrap());
                }
                tray::acceptance::choose_last_action().unwrap();
                assert_eq!(observed.recv_timeout(Duration::from_secs(2)).unwrap(), 3);
            });
            proxy.open_menu();
            tray.run(|event, proxy| {
                if let tray::Event::Command(id) = event {
                    command.send(id).unwrap();
                    proxy.close();
                }
            })
            .unwrap();
            photographer.join().unwrap();
            let text = serde_json::to_string(&sample).unwrap();
            tray::copy_text(&text).unwrap();
        }
        fs::write(
            artifacts.join("result.json"),
            "{\"native_popup\":true,\"real_provider\":true,\"native_keyboard_quit\":true,\"open_menu_disconnect_clears_all_12\":true,\"copy_disabled_offline\":true,\"locales\":5,\"themes\":4}\n",
        )
        .unwrap();
    }

    fn analysis_windows(socket: &Path, artifacts: &Path) {
        use nddev_systeminfo::analysis::{Selection, Store};
        let mut client =
            Client::connect_analysis(socket, 0, Duration::from_secs(5), false).unwrap();
        let mut store = Store::default();
        for _ in 0..3 {
            store.push(client.next_analysis().unwrap().unwrap());
        }
        let cpu = store.view(&Selection::parse("cpu.usage").unwrap(), 3600, 512);
        let id = cpu.latest.as_ref().unwrap().detail.cores[0].id;
        let core_key = format!("core:{id}");
        let core = store.view(&Selection::parse(&core_key).unwrap(), 3600, 512);
        let resources = || {
            let r = tray::acceptance::process_resources().unwrap();
            serde_json::json!({"working_set_bytes":r.working_set_bytes,"private_commit_bytes":r.private_commit_bytes,"gdi_handles":r.gdi_handles,"user_handles":r.user_handles})
        };
        let baseline = resources();
        let mut observations = Vec::new();
        for (code, theme) in [
            ("en", "light"),
            ("ru", "dark"),
            ("kk", "light-contrast"),
            ("ar", "dark-contrast"),
            ("zh", "light"),
        ] {
            let mut config = Config::parse(nddev_systeminfo::config::DEFAULT_TOML).unwrap();
            config.display.locale = code.into();
            config.display.theme = theme.into();
            let tray = Tray::new(
                &format!(
                    "org.nddev.systeminfo.analysis.acceptance.{}.{code}",
                    std::process::id()
                ),
                nddev_systeminfo::windows_ui::native_model(
                    &config,
                    Some(&cpu.latest.as_ref().unwrap().sample),
                ),
            )
            .unwrap()
            .unwrap();
            tray::acceptance::focus(&tray).unwrap();
            let proxy = tray.proxy();
            let analysis = nddev_systeminfo::windows_ui::native_analysis_model(
                &config,
                &cpu,
                Some("cpu.usage"),
            );
            let expected_labels: Vec<_> = analysis
                .rows
                .iter()
                .filter_map(|row| {
                    if let tray::dashboard::Row::Text(text) = row {
                        Some(text.replace(['\u{2068}', '\u{2069}'], "\u{200e}"))
                    } else {
                        None
                    }
                })
                .collect();
            assert!(!expected_labels.is_empty());
            proxy.dashboard(Some(analysis));
            let capture = proxy.clone();
            let path = artifacts.join(format!("analysis-{code}-{theme}.png"));
            let core_path = artifacts.join(format!("core-{code}-{theme}.png"));
            let (sender, receiver) = mpsc::sync_channel(2);
            let photographer = thread::spawn(move || {
                let _close = Close(capture.clone());
                let deadline = Instant::now() + Duration::from_secs(15);
                while !tray::acceptance::capture_dashboard(&path).unwrap() {
                    assert!(
                        Instant::now() < deadline,
                        "native analysis window did not become visible"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                assert_eq!(
                    tray::acceptance::dashboard_labels().unwrap(),
                    expected_labels
                );
                let opened = resources();
                assert!(tray::acceptance::click_dashboard(11000).unwrap());
                assert_eq!(
                    receiver.recv_timeout(Duration::from_secs(3)).unwrap(),
                    11000
                );
                // The event handler publishes the actual core projection before
                // acknowledging the action. Capture after that HWND update.
                thread::sleep(Duration::from_millis(100));
                assert!(tray::acceptance::capture_dashboard(&core_path).unwrap());
                assert!(tray::acceptance::click_dashboard(tray::dashboard::CLOSED).unwrap());
                assert_eq!(
                    receiver.recv_timeout(Duration::from_secs(3)).unwrap(),
                    tray::dashboard::CLOSED
                );
                while tray::acceptance::dashboard_visible() {
                    assert!(Instant::now() < deadline, "closed analysis HWND leaked");
                    thread::sleep(Duration::from_millis(10));
                }
                opened
            });
            tray.run(|event, proxy| {
                if let tray::Event::Command(id) = event {
                    match id {
                        11000 => proxy.dashboard(Some(
                            nddev_systeminfo::windows_ui::native_analysis_model(
                                &config,
                                &core,
                                Some(&core_key),
                            ),
                        )),
                        tray::dashboard::CLOSED => proxy.dashboard(None),
                        _ => panic!("unexpected analysis command {id}"),
                    }
                    sender.send(id).unwrap();
                }
            })
            .unwrap();
            let opened = photographer.join().unwrap();
            observations.push(serde_json::json!({"locale":code,"open":opened,"after_window_and_tray_close":resources()}));
        }
        fs::write(artifacts.join("resources.json"), serde_json::to_vec_pretty(&serde_json::json!({"scope":"native-control-surface","baseline":baseline,"observations":observations})).unwrap()).unwrap();
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

//! Standalone CLI; stdout samples remain a stable machine interface.
use nddev_systeminfo::{
    client::Client,
    config::{self, Config, LegacySettings},
    protocol,
};
use nddev_systeminfo_native::transport;
use std::{
    error::Error,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let (mut config_path, mut socket, mut interval) = (None, None, None);
    let (mut once, mut json) = (false, false);
    let mut legacy = None;
    let mut command = Vec::new();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                println!(
                    "NDDev Systeminfo {}\n\nnddev-systeminfo [sample|watch] [--once] [--socket PATH] [--interval 0..60]\nnddev-systeminfo config <show|path|check|init|set KEY VALUE> [--json]\nnddev-systeminfo schema\n\n--config FILE selects a complete TOML configuration.\n0 requests realtime (500 ms). Missing readings remain null.\nNative collection requires Linux, macOS or Windows; NDS is optional.",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            "--version" | "-V" => {
                println!("nddev-systeminfo {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--config" => {
                config_path = Some(PathBuf::from(args.next().ok_or("--config needs a file")?))
            }
            "--socket" => {
                socket = Some(PathBuf::from(
                    args.next().ok_or("--socket needs an absolute path")?,
                ))
            }
            "--interval" => {
                interval = Some(
                    args.next()
                        .ok_or("--interval needs a number")?
                        .parse::<u64>()?,
                )
            }
            "--once" => once = true,
            "--json" => json = true,
            "--legacy-json" => legacy = Some(args.next().ok_or("--legacy-json needs an object")?),
            option if option.starts_with('-') => {
                return Err(format!("unknown option: {option}").into());
            }
            _ => command.push(argument),
        }
    }
    let operation = command.first().map(String::as_str).unwrap_or("sample");
    if operation == "schema" {
        println!("{}", protocol::SCHEMA);
        return Ok(());
    }
    let mut config = if operation == "config"
        && command.get(1).is_some_and(|value| {
            value == "init"
                || value == "migrate" && config_path.as_ref().is_some_and(|path| !path.exists())
        }) {
        Config::parse(config::DEFAULT_TOML)?
    } else {
        Config::load(config_path.as_deref())?
    };
    if operation == "config" {
        let path = config_path.map(Ok).unwrap_or_else(config::default_path)?;
        match command.get(1).map(String::as_str).unwrap_or("show") {
            "path" => println!("{}", path.display()),
            "show" if json => println!("{}", serde_json::to_string_pretty(&config)?),
            "show" => print!("{}", toml::to_string_pretty(&config)?),
            "check" => println!("{{\"valid\":true}}"),
            "init" => {
                if !config.store_if_absent(&path)? {
                    return Err("configuration already exists".into());
                }
                println!("{}", path.display());
            }
            "migrate" => {
                let legacy: LegacySettings = serde_json::from_str(
                    legacy.as_deref().ok_or("migration needs --legacy-json")?,
                )?;
                let migrated = legacy.apply(&mut config)? && config.store_if_absent(&path)?;
                println!("{{\"migrated\":{migrated}}}");
            }
            "set" => {
                if command.len() != 4 {
                    return Err("config set needs KEY VALUE".into());
                }
                config.set(&command[2], &command[3])?;
                config.store(&path)?;
                println!("{{\"saved\":true,\"applies\":\"next_start\"}}");
            }
            _ => return Err("unknown config command".into()),
        }
        return Ok(());
    }
    if command.len() > 1 || !["sample", "watch"].contains(&operation) {
        return Err("unknown command; use --help".into());
    }
    let path = socket.map(Ok).unwrap_or_else(transport::socket_path)?;
    let interval = interval.unwrap_or(config.sampling.interval_seconds);
    let mut client = Client::connect(
        &path,
        interval,
        Duration::from_millis(config.transport.client_timeout_ms),
    )?;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    loop {
        let sample = client.next_sample()?;
        serde_json::to_writer(&mut output, &sample)?;
        output.write_all(b"\n")?;
        output.flush()?;
        if once || operation == "sample" {
            return Ok(());
        }
    }
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::BrokenPipe)
                || error
                    .downcast_ref::<serde_json::Error>()
                    .is_some_and(|error| error.io_error_kind() == Some(io::ErrorKind::BrokenPipe))
            {
                return ExitCode::SUCCESS;
            }
            eprintln!("nddev-systeminfo: {error}");
            ExitCode::FAILURE
        }
    }
}

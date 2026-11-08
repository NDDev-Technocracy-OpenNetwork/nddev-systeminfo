//! Background daemon; all collection is shared and demand-driven.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod server;
use nddev_systeminfo::{config::Config, telemetry};
use nddev_systeminfo_native::transport;
use std::{path::PathBuf, process::ExitCode};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (mut configuration, mut socket) = (None, None);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--version" | "-V" => {
                println!("nddev-systeminfod {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "NDDev Systeminfo {}\n\nnddev-systeminfod [--config FILE] [--socket ABSOLUTE_PATH]\n\nConfiguration: nddev-systeminfo config show\nProtocol: nddev-systeminfo schema",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            "--config" => {
                configuration = Some(PathBuf::from(args.next().ok_or("--config needs a file")?))
            }
            "--socket" => socket = Some(PathBuf::from(args.next().ok_or("--socket needs a path")?)),
            _ => return Err(format!("unknown argument: {argument}").into()),
        }
    }
    let config = Config::load(configuration.as_deref())?;
    telemetry::init(&config.logging.level);
    let (listener, activated) = transport::bind(socket.as_deref())?;
    tracing::info!(
        event.name = "systeminfo.started",
        service.name = "nddev-systeminfod",
        version = env!("CARGO_PKG_VERSION"),
        gpu = config.sampling.gpu
    );
    server::serve(listener, activated, &config)?;
    tracing::info!(event.name = "systeminfo.stopped", reason = "idle");
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            telemetry::failure(&error.to_string());
            ExitCode::FAILURE
        }
    }
}

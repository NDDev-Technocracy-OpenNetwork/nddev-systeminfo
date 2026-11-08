//! Release-only measurements against the real Linux provider. No synthetic
//! collectors, altered driver libraries, background polling or machine inventory.
use crate::Result;

pub(super) fn measure() -> Result {
    #[cfg(target_os = "linux")]
    {
        linux::measure()
    }
    #[cfg(not(target_os = "linux"))]
    Err("Linux resource acceptance needs procfs; use native OS process tools elsewhere".into())
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use serde_json::{Value, json};
    use std::{
        fs,
        io::{BufRead, BufReader, Write},
        os::unix::net::UnixStream,
        path::{Path, PathBuf},
        process::{Child, Command},
        thread,
        time::{Duration, Instant},
    };
    struct Run {
        child: Child,
        directory: PathBuf,
    }
    impl Drop for Run {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
    fn process(pid: u32) -> Result<Value> {
        let directory = PathBuf::from(format!("/proc/{pid}"));
        let status = fs::read_to_string(directory.join("status"))?;
        let memory = fs::read_to_string(directory.join("smaps_rollup"))?;
        let number = |source: &str, key: &str| -> Result<u64> {
            source
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .and_then(|value| value.split_whitespace().next())
                .ok_or_else(|| format!("procfs field absent: {key}").into())
                .and_then(|value| Ok(value.parse()?))
        };
        let stat = fs::read_to_string(directory.join("stat"))?;
        let fields: Vec<&str> = stat
            .rsplit_once(')')
            .ok_or("invalid proc stat")?
            .1
            .split_whitespace()
            .collect();
        Ok(json!({
            "pss_kib": number(&memory,"Pss:")?,
            "private_kib": number(&memory,"Private_Clean:")? + number(&memory,"Private_Dirty:")?,
            "threads": number(&status,"Threads:")?,
            "descriptors": fs::read_dir(directory.join("fd"))?.count(),
            "cpu_ticks": fields[11].parse::<u64>()? + fields[12].parse::<u64>()?
        }))
    }
    pub(super) fn measure() -> Result {
        let binaries = crate::build_binaries(true)?;
        let provider = &binaries["nddev-systeminfod"];
        let output = Path::new(".tools/resources");
        fs::create_dir_all(output)?;
        let mut measurements = Vec::new();
        for gpu in [false, true] {
            let directory =
                std::env::temp_dir().join(format!("si-measure-{}-{gpu}", std::process::id()));
            fs::create_dir(&directory)?;
            let path = directory.join("s");
            let config = directory.join("config.toml");
            let mut settings: toml::Value =
                toml::from_str(&fs::read_to_string("config/default.toml")?)?;
            settings["sampling"]["gpu"] = toml::Value::Boolean(gpu);
            let idle = settings["sampling"]["idle_release_seconds"]
                .as_integer()
                .ok_or("idle interval missing")? as u64;
            fs::write(&config, toml::to_string(&settings)?)?;
            let log = fs::File::create(output.join(format!("provider-gpu-{gpu}.jsonl")))?;
            let child = Command::new(provider)
                .args(["--socket"])
                .arg(&path)
                .arg("--config")
                .arg(&config)
                .stdout(log.try_clone()?)
                .stderr(log)
                .spawn()?;
            let mut run = Run { child, directory };
            let pid = run.child.id();
            let ready = Instant::now() + Duration::from_secs(10);
            while !path.exists() {
                if run.child.try_wait()?.is_some() || Instant::now() > ready {
                    return Err("provider did not become ready".into());
                }
                thread::sleep(Duration::from_millis(2));
            }
            let cold = process(pid)?;
            let start = Instant::now();
            let mut stream = UnixStream::connect(&path)?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            stream.write_all(b"{\"interval\":0}\n")?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line)?;
            let first_frame_ms = start.elapsed().as_secs_f64() * 1000.0;
            let frame: Value = serde_json::from_str(&line)?;
            if frame["v"] != 1 {
                return Err("unexpected snapshot version".into());
            }
            let active_start = process(pid)?;
            let until = Instant::now() + Duration::from_secs(10);
            while Instant::now() < until {
                line.clear();
                if reader.read_line(&mut line)? == 0 {
                    return Err("provider closed active client".into());
                }
            }
            let active = process(pid)?;
            drop(reader);
            println!(
                "GPU={gpu}: active readings measured; waiting for collector release ({idle}s)"
            );
            thread::sleep(Duration::from_secs(idle + 2));
            let released = process(pid)?;
            thread::sleep(Duration::from_secs(10));
            let idle_end = process(pid)?;
            let before = Instant::now();
            let mut stream = UnixStream::connect(&path)?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            stream.write_all(b"{\"interval\":0}\n")?;
            line.clear();
            BufReader::new(stream).read_line(&mut line)?;
            let _: Value = serde_json::from_str(&line)?;
            measurements.push(json!({"gpu":gpu,"cold":cold,"first_frame_ms":first_frame_ms,
                "active":active,"active_cpu_ticks_10s":active["cpu_ticks"].as_u64().unwrap()-active_start["cpu_ticks"].as_u64().unwrap(),
                "after_release":released,"idle_cpu_ticks_10s":idle_end["cpu_ticks"].as_u64().unwrap()-released["cpu_ticks"].as_u64().unwrap(),
                "reconnect_first_frame_ms":before.elapsed().as_secs_f64()*1000.0}));
            println!("GPU={gpu}: idle and reconnect measured");
        }
        let result = json!({"version":crate::version()?,"profile":"release","interval_seconds":0,"active_seconds":10,"measurements":measurements});
        fs::write(
            output.join("result.json"),
            serde_json::to_string_pretty(&result)? + "\n",
        )?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }
}

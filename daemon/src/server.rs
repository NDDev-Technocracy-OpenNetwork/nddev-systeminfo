//! One demand-driven hardware sampler. An idle resident owns no collector.
use nddev_systeminfo::config::{Config, Transport, cadence};
use nddev_systeminfo_native::{
    Collector,
    transport::{UnixListener, UnixStream},
};
use std::{
    io::{self, Read, Write},
    sync::mpsc::{self, Receiver, RecvTimeoutError, TrySendError},
    time::{Duration, Instant},
};

pub(super) fn serve(listener: UnixListener, activated: bool, config: &Config) -> io::Result<()> {
    let clients = accept_loop(listener, config.transport)?;
    let mut connected = Vec::with_capacity(config.transport.max_clients);
    let mut sampler = Sampler {
        collector: None,
        sampled_at: None,
        line: String::with_capacity(512),
        gpu: config.sampling.gpu,
    };
    let configured = config.interval();
    let idle_limit = Duration::from_secs(config.sampling.idle_release_seconds);
    let mut idle_since = Instant::now();
    loop {
        let next = if connected.is_empty() {
            if sampler.collector.is_none() && !activated {
                None
            } else {
                Some(idle_limit.saturating_sub(idle_since.elapsed()))
            }
        } else {
            Some(
                requested_cadence(&connected, configured).saturating_sub(
                    sampler
                        .sampled_at
                        .expect("connected clients have a sample")
                        .elapsed(),
                ),
            )
        };
        let incoming = match next {
            Some(wait) => clients.recv_timeout(wait),
            None => clients.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match incoming {
            Ok(client) => sampler.admit(
                client,
                &mut connected,
                config.transport.max_clients,
                configured,
            )?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other("accept thread stopped"));
            }
        }
        for _ in 1..config.transport.max_clients {
            let Ok(client) = clients.try_recv() else {
                break;
            };
            sampler.admit(
                client,
                &mut connected,
                config.transport.max_clients,
                configured,
            )?;
        }
        if !connected.is_empty()
            && sampler
                .sampled_at
                .is_none_or(|at| at.elapsed() >= requested_cadence(&connected, configured))
        {
            sampler.refresh(&mut connected)?;
        }
        if connected.is_empty() {
            if idle_since.elapsed() >= idle_limit {
                if activated {
                    return Ok(());
                }
                // Drop NVML, ports and open procfs/sysfs handles before blocking.
                sampler.release();
            }
        } else {
            idle_since = Instant::now();
        }
    }
}

struct Sampler {
    collector: Option<Collector>,
    sampled_at: Option<Instant>,
    line: String,
    gpu: bool,
}
impl Sampler {
    fn refresh(&mut self, connected: &mut Vec<Client>) -> io::Result<()> {
        if self.collector.is_none() {
            self.collector = Some(Collector::new(self.gpu)?);
            tracing::debug!(event.name = "systeminfo.collector.started");
        }
        self.collector
            .as_mut()
            .expect("initialized collector")
            .sample()
            .encode(&mut self.line);
        self.sampled_at = Some(Instant::now());
        connected.retain_mut(|client| client.stream.write_all(self.line.as_bytes()).is_ok());
        Ok(())
    }
    fn admit(
        &mut self,
        client: Client,
        connected: &mut Vec<Client>,
        limit: usize,
        configured: Duration,
    ) -> io::Result<()> {
        if connected.len() == limit {
            return Ok(());
        }
        let wanted = connected
            .iter()
            .filter_map(|client| client.requested)
            .chain(client.requested)
            .min()
            .unwrap_or(configured);
        if self.sampled_at.is_none_or(|at| at.elapsed() >= wanted) {
            self.refresh(connected)?;
        }
        admit(client, connected, self.line.as_bytes(), limit);
        Ok(())
    }
    fn release(&mut self) {
        if self.collector.is_some() {
            tracing::debug!(event.name = "systeminfo.collector.released");
        }
        self.collector = None;
        self.sampled_at = None;
        self.line.clear();
    }
}
struct Client {
    stream: UnixStream,
    requested: Option<Duration>,
}
fn admit(mut client: Client, connected: &mut Vec<Client>, latest: &[u8], limit: usize) {
    if connected.len() < limit && client.stream.write_all(latest).is_ok() {
        connected.push(client);
    }
}
fn requested_cadence(clients: &[Client], configured: Duration) -> Duration {
    clients
        .iter()
        .filter_map(|client| client.requested)
        .min()
        .unwrap_or(configured)
}
fn handshake(mut stream: &UnixStream, config: Transport) -> io::Result<Option<Duration>> {
    let deadline = Instant::now() + Duration::from_millis(config.handshake_timeout_ms);
    let mut bytes = [0u8; nddev_systeminfo::protocol::MAX_FRAME_BYTES];
    let mut used = 0;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        stream.set_read_timeout(Some(remaining))?;
        match stream.read(&mut bytes[used..config.handshake_bytes]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(count) => {
                used += count;
                if let Some(end) = bytes[..used].iter().position(|byte| *byte == b'\n') {
                    return Ok(std::str::from_utf8(&bytes[..end])
                        .ok()
                        .and_then(parse_interval));
                }
                if used == config.handshake_bytes {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "handshake exceeds configured limit",
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
    }
}
fn parse_interval(line: &str) -> Option<Duration> {
    let object = line.trim().strip_prefix('{')?.strip_suffix('}')?;
    let (key, value) = object.split_once(':')?;
    let value = value.trim();
    if key.trim() != "\"interval\""
        || value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    cadence(value.parse().ok()?)
}
fn accept_loop(listener: UnixListener, config: Transport) -> io::Result<Receiver<Client>> {
    let (sender, receiver) = mpsc::sync_channel(config.max_clients);
    std::thread::Builder::new().name("accept".into()).stack_size(config.accept_stack_bytes).spawn(move || loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => { tracing::error!(event.name = "systeminfo.accept.failed", error.type = "io", error = %error); return; },
        };
        let Ok(requested) = handshake(&stream, config) else { continue; };
        if stream.set_nonblocking(true).is_err() { continue; }
        match sender.try_send(Client { stream, requested }) { Ok(()) | Err(TrySendError::Full(_)) => {}, Err(TrySendError::Disconnected(_)) => return }
    })?;
    Ok(receiver)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config::parse(nddev_systeminfo::config::DEFAULT_TOML).unwrap()
    }
    #[test]
    fn cadence_accepts_only_bounded_whole_seconds() {
        for (text, seconds) in [
            (r#"{"interval":0}"#, 0),
            (r#"{ "interval" : 12 }"#, 12),
            (r#"{"interval":60}"#, 60),
        ] {
            assert_eq!(parse_interval(text), cadence(seconds));
        }
        for text in [
            "",
            "hello",
            r#"{"interval":61}"#,
            r#"{"interval":-1}"#,
            r#"{"interval":1.5}"#,
            r#"{"interval":1e2}"#,
            r#"{"interval":00}"#,
            r#"{"interval":"5"}"#,
            r#"garbage "interval":0"#,
            r#"{"interval":3,"other":4}"#,
        ] {
            assert_eq!(parse_interval(text), None, "{text}");
        }
    }
    #[test]
    fn oversized_handshake_is_rejected_without_allocating() {
        let (server, mut peer) = UnixStream::pair().unwrap();
        let config = config().transport;
        peer.write_all(&vec![b'x'; config.handshake_bytes]).unwrap();
        assert_eq!(
            handshake(&server, config).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[test]
    fn client_limit_and_fastest_cadence_are_enforced() {
        let mut connected = Vec::new();
        let mut peers = Vec::new();
        for seconds in [10, 2, 0] {
            let (stream, peer) = UnixStream::pair().unwrap();
            peers.push(peer);
            admit(
                Client {
                    stream,
                    requested: cadence(seconds),
                },
                &mut connected,
                b"frame\n",
                2,
            );
        }
        assert_eq!(connected.len(), 2);
        assert_eq!(
            requested_cadence(&connected, config().interval()),
            Duration::from_secs(2)
        );
    }
}

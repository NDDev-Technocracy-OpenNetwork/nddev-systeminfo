//! One demand-driven sampler; replay IO never blocks hardware publication.
use nddev_systeminfo::{
    config::{Config, Transport, cadence},
    history::{History, Record},
    protocol::analysis::Details,
};
use nddev_systeminfo_native::{
    Collector,
    transport::{UnixListener, UnixStream},
};
use std::{
    io::{self, Read, Write},
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) fn serve(listener: UnixListener, activated: bool, config: &Config) -> io::Result<()> {
    let (clients, sender) = accept_loop(listener, config.transport)?;
    let mut connected = Vec::with_capacity(config.transport.max_clients);
    let mut sampler = Sampler {
        collector: None,
        sampled_at: None,
        line: String::with_capacity(512),
        detail_line: String::new(),
        gpu: config.sampling.gpu,
        history: History::default(),
        timeline_ms: 0,
        wall: None,
        sequence: 0,
        next_replay: 0,
        replays: Vec::new(),
        sender,
        write_timeout: Duration::from_millis(config.transport.client_timeout_ms),
        stack: config.transport.accept_stack_bytes,
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
            Ok(event) => sampler.admit(
                event,
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
            let Ok(event) = clients.try_recv() else {
                break;
            };
            sampler.admit(
                event,
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
            sampler.refresh(&mut connected, configured)?;
        }
        if connected.is_empty() && sampler.replays.is_empty() {
            if idle_since.elapsed() >= idle_limit {
                if activated {
                    return Ok(());
                }
                sampler.release();
            }
        } else {
            idle_since = Instant::now();
        }
    }
}
#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Request {
    interval: Option<u64>,
    details: bool,
    history: bool,
    record: bool,
}
impl Request {
    fn cadence(self) -> Option<Duration> {
        self.interval.and_then(cadence)
    }
}
struct Client {
    stream: UnixStream,
    request: Request,
    replayed: bool,
    last_sequence: u64,
}
enum Admission {
    Incoming(Client),
    Replayed(u64, io::Result<Client>),
    Failed(io::Error),
}
struct Sampler {
    collector: Option<Collector>,
    sampled_at: Option<Instant>,
    line: String,
    detail_line: String,
    gpu: bool,
    history: History,
    timeline_ms: u64,
    wall: Option<SystemTime>,
    sequence: u64,
    next_replay: u64,
    replays: Vec<(u64, JoinHandle<()>)>,
    sender: SyncSender<Admission>,
    write_timeout: Duration,
    stack: usize,
}
impl Sampler {
    fn refresh(&mut self, connected: &mut Vec<Client>, configured: Duration) -> io::Result<()> {
        let wall = SystemTime::now();
        let at = Instant::now();
        if let (Some(previous_wall), Some(previous_at)) = (self.wall, self.sampled_at) {
            match wall.duration_since(previous_wall) {
                Ok(real) => {
                    let awake = at.saturating_duration_since(previous_at);
                    self.timeline_ms = self.timeline_ms.saturating_add(
                        real.max(awake).as_millis().min(u128::from(u64::MAX)) as u64,
                    );
                    // Instant is not suspend-inclusive on every OS. Re-prime
                    // counters when wall/awake clocks reveal sleep or a step.
                    if real.abs_diff(awake) > Duration::from_secs(2) {
                        self.collector = None;
                    }
                }
                Err(_) => {
                    self.history.clear();
                    self.timeline_ms = 0;
                    self.sequence = 0;
                    self.collector = None;
                }
            }
        }
        self.wall = Some(wall);
        if self.collector.is_none() {
            self.collector = Some(Collector::new(self.gpu)?);
            tracing::debug!(event.name = "systeminfo.collector.started");
        }
        let collector = self.collector.as_mut().expect("initialized collector");
        let snapshot = collector.sample();
        let at = Instant::now();
        self.sequence += 1;
        let detail = Details {
            at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            elapsed_ms: self.timeline_ms,
            sequence: self.sequence,
            period_ms: requested_cadence(connected, configured).as_millis() as u64,
            gpu_enabled: self.gpu,
            cores: collector.cores().to_vec(),
            gpus: collector.gpus().to_vec(),
        };
        detail.validate().map_err(io::Error::other)?;
        if connected.iter().any(|c| c.request.details) {
            detail
                .encode(snapshot, &mut self.detail_line)
                .map_err(io::Error::other)?;
        }
        snapshot.encode(&mut self.line);
        let record = Record::new(snapshot, detail, at);
        if connected
            .iter()
            .any(|client| client.request.record || client.request.history)
            || !self.replays.is_empty()
        {
            self.history.push(record);
        } else {
            self.history.replace_latest(record);
        }
        self.sampled_at = Some(at);
        connected.retain_mut(|client| {
            let bytes = if client.request.details {
                self.detail_line.as_bytes()
            } else {
                self.line.as_bytes()
            };
            if client.stream.write_all(bytes).is_ok() {
                client.last_sequence = self.sequence;
                true
            } else {
                false
            }
        });
        Ok(())
    }
    fn admit(
        &mut self,
        event: Admission,
        connected: &mut Vec<Client>,
        limit: usize,
        configured: Duration,
    ) -> io::Result<()> {
        let mut client = match event {
            Admission::Incoming(client) => client,
            Admission::Failed(error) => return Err(error),
            Admission::Replayed(id, result) => {
                if let Some(index) = self.replays.iter().position(|(old, _)| *old == id) {
                    self.replays
                        .swap_remove(index)
                        .1
                        .join()
                        .map_err(|_| io::Error::other("history writer failed"))?;
                }
                let Ok(client) = result else {
                    return Ok(());
                };
                client
            }
        };
        if connected.len() + self.replays.len() >= limit {
            return Ok(());
        }
        let wanted = connected
            .iter()
            .filter_map(|c| c.request.cadence())
            .chain(client.request.cadence())
            .min()
            .unwrap_or(configured);
        if self.due(wanted) {
            self.refresh(connected, configured)?;
        }
        if client.request.history && !client.replayed {
            let records = self.history.snapshot();
            let sender = self.sender.clone();
            let timeout = self.write_timeout;
            self.next_replay = self.next_replay.wrapping_add(1);
            let id = self.next_replay;
            let worker = std::thread::Builder::new()
                .name("history-replay".into())
                .stack_size(self.stack.max(128 * 1024))
                .spawn(move || {
                    let result = replay(client, records, timeout);
                    let _ = sender.send(Admission::Replayed(id, result));
                })?;
            self.replays.push((id, worker));
            return Ok(());
        }
        if client.last_sequence != self.sequence {
            if client.request.details {
                let record = self.history.latest().expect("a completed sample exists");
                record
                    .detail
                    .encode(record.sample, &mut self.detail_line)
                    .map_err(io::Error::other)?;
            }
            let bytes = if client.request.details {
                self.detail_line.as_bytes()
            } else {
                self.line.as_bytes()
            };
            if client.stream.write_all(bytes).is_err() {
                return Ok(());
            }
            client.last_sequence = self.sequence;
        }
        connected.push(client);
        Ok(())
    }
    fn due(&self, wanted: Duration) -> bool {
        self.sampled_at.is_none_or(|at| at.elapsed() >= wanted)
            || self
                .wall
                .is_some_and(|at| !at.elapsed().is_ok_and(|elapsed| elapsed < wanted))
    }
    fn release(&mut self) {
        if self.collector.is_some() {
            tracing::debug!(event.name = "systeminfo.collector.released");
        }
        self.collector = None;
        self.sampled_at = None;
        self.wall = None;
        self.timeline_ms = 0;
        self.sequence = 0;
        self.line.clear();
        self.detail_line = String::new();
        self.history.clear();
    }
}
fn replay(mut client: Client, records: Vec<Arc<Record>>, timeout: Duration) -> io::Result<Client> {
    client.stream.set_nonblocking(false)?;
    let deadline = Instant::now() + timeout;
    let mut line = String::new();
    for record in records {
        let Some(remaining) = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
        else {
            return Err(io::ErrorKind::TimedOut.into());
        };
        client.stream.set_write_timeout(Some(remaining))?;
        record
            .detail
            .encode(record.sample, &mut line)
            .map_err(io::Error::other)?;
        client.stream.write_all(line.as_bytes())?;
        client.last_sequence = record.detail.sequence;
    }
    let Some(remaining) = deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
    else {
        return Err(io::ErrorKind::TimedOut.into());
    };
    client.stream.set_write_timeout(Some(remaining))?;
    client.stream.write_all(b"{\"history_end\":true}\n")?;
    client.stream.set_nonblocking(true)?;
    client.replayed = true;
    Ok(client)
}
#[cfg(test)]
fn admit(mut client: Client, connected: &mut Vec<Client>, latest: &[u8], limit: usize) {
    if connected.len() < limit && client.stream.write_all(latest).is_ok() {
        connected.push(client);
    }
}
fn requested_cadence(clients: &[Client], configured: Duration) -> Duration {
    clients
        .iter()
        .filter_map(|client| client.request.cadence())
        .min()
        .unwrap_or(configured)
}
fn handshake(mut stream: &UnixStream, config: Transport) -> io::Result<Request> {
    let deadline = Instant::now() + Duration::from_millis(config.handshake_timeout_ms);
    let mut bytes = [0u8; nddev_systeminfo::protocol::MAX_FRAME_BYTES];
    let mut used = 0;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(Request::default());
        }
        stream.set_read_timeout(Some(remaining))?;
        match stream.read(&mut bytes[used..config.handshake_bytes]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(count) => {
                used += count;
                if let Some(end) = bytes[..used].iter().position(|b| *b == b'\n') {
                    return Ok(std::str::from_utf8(&bytes[..end])
                        .ok()
                        .and_then(parse_request)
                        .unwrap_or_default());
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
                return Ok(Request::default());
            }
            Err(error) => return Err(error),
        }
    }
}
fn parse_request(line: &str) -> Option<Request> {
    let request: Request = serde_json::from_str(line).ok()?;
    if request.interval.is_some_and(|n| cadence(n).is_none()) || request.history && !request.details
    {
        return None;
    }
    Some(request)
}
#[cfg(test)]
fn parse_interval(line: &str) -> Option<Duration> {
    parse_request(line)?.cadence()
}
fn accept_loop(
    listener: UnixListener,
    config: Transport,
) -> io::Result<(Receiver<Admission>, SyncSender<Admission>)> {
    let (sender, receiver) = mpsc::sync_channel(config.max_clients);
    let accepted = sender.clone();
    std::thread::Builder::new()
        .name("accept".into())
        .stack_size(config.accept_stack_bytes)
        .spawn(move || {
            loop {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let _ = sender.send(Admission::Failed(error));
                        return;
                    }
                };
                let Ok(request) = handshake(&stream, config) else {
                    continue;
                };
                if stream.set_nonblocking(true).is_err() {
                    continue;
                }
                match sender.try_send(Admission::Incoming(Client {
                    stream,
                    request,
                    replayed: false,
                    last_sequence: 0,
                })) {
                    Ok(()) | Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Disconnected(_)) => return,
                }
            }
        })?;
    Ok((receiver, accepted))
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
            handshake(&server, config).err().unwrap().kind(),
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
                    request: Request {
                        interval: Some(seconds),
                        ..Request::default()
                    },
                    replayed: false,
                    last_sequence: 0,
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

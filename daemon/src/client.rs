//! Bounded blocking client; one connection has one reader and an absolute deadline.
use crate::{
    config::cadence,
    protocol::{self, WireSnapshot},
};
use socket2::{Domain, SockAddr, Socket, Type};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("systeminfo provider is unavailable: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Protocol(#[from] protocol::ProtocolError),
    #[error("invalid client interval or timeout")]
    InvalidConfiguration,
}

struct DeadlineReader {
    socket: Arc<Socket>,
    deadline: Instant,
}
impl Read for DeadlineReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or(io::ErrorKind::TimedOut)?;
        if let Err(timeout_error) = self.socket.set_read_timeout(Some(remaining)) {
            // Darwin may reject timeout setup after a peer closed even when
            // its last frame is readable. Probe without ever blocking.
            self.socket.set_nonblocking(true)?;
            let result = self.socket.as_ref().read(bytes);
            self.socket.set_nonblocking(false)?;
            if Instant::now() >= self.deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            return match result {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => Err(timeout_error),
                result => result,
            };
        }
        let result = self.socket.as_ref().read(bytes);
        if Instant::now() >= self.deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        result
    }
}

pub struct Client {
    reader: BufReader<DeadlineReader>,
    buffer: Vec<u8>,
    timeout: Duration,
}
impl Client {
    /// Share the original socket rather than duplicate a Winsock handle. Both
    /// cancellation-before-read and cancellation-during-read shut down the same
    /// native handle; the last owner closes it after the reader has returned.
    pub fn cancellation(&self) -> Cancellation {
        Cancellation(self.reader.get_ref().socket.clone())
    }
    pub fn connect(path: &Path, interval: u64, timeout: Duration) -> Result<Self, ClientError> {
        Self::open(path, interval, timeout, None, false)
    }
    /// Ordinary v1 monitoring with explicit one-hour history retention.
    pub fn connect_recording(
        path: &Path,
        interval: u64,
        timeout: Duration,
    ) -> Result<Self, ClientError> {
        Self::open(path, interval, timeout, None, true)
    }
    /// Negotiate larger detail frames and an optional history replay. Ordinary
    /// v1 clients keep their original handshake, buffer and validation limits.
    pub fn connect_analysis(
        path: &Path,
        interval: u64,
        timeout: Duration,
        history: bool,
    ) -> Result<Self, ClientError> {
        Self::open(path, interval, timeout, Some(history), true)
    }
    fn open(
        path: &Path,
        interval: u64,
        timeout: Duration,
        history: Option<bool>,
        record: bool,
    ) -> Result<Self, ClientError> {
        let cadence = cadence(interval).ok_or(ClientError::InvalidConfiguration)?;
        if !path.is_absolute() || timeout.is_zero() || timeout > Duration::from_secs(60) {
            return Err(ClientError::InvalidConfiguration);
        }
        let stream = Socket::new(Domain::UNIX, Type::STREAM, None)?;
        stream.connect_timeout(&SockAddr::unix(path)?, timeout)?;
        stream.set_write_timeout(Some(timeout))?;
        if let Some(history) = history {
            writeln!(
                &stream,
                "{{\"interval\":{interval},\"details\":true,\"history\":{history},\"record\":true}}"
            )?;
        } else if record {
            writeln!(&stream, "{{\"interval\":{interval},\"record\":true}}")?;
        } else {
            writeln!(&stream, "{{\"interval\":{interval}}}")?;
        }
        Ok(Self {
            reader: BufReader::with_capacity(
                1024,
                DeadlineReader {
                    socket: Arc::new(stream),
                    deadline: Instant::now() + timeout,
                },
            ),
            buffer: Vec::with_capacity(512),
            timeout: timeout + cadence,
        })
    }
    pub fn next_sample(&mut self) -> Result<WireSnapshot, ClientError> {
        Ok(protocol::decode(
            self.read_line(protocol::MAX_FRAME_BYTES)?,
        )?)
    }
    /// None marks the end of initial history. Socket EOF is an IO error.
    pub fn next_analysis(&mut self) -> Result<Option<protocol::analysis::Frame>, ClientError> {
        let bytes = self.read_line(protocol::analysis::MAX_ANALYSIS_FRAME_BYTES)?;
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Boundary {
            history_end: bool,
        }
        if !bytes.starts_with(b"{\"v\":")
            && serde_json::from_slice::<Boundary>(bytes).is_ok_and(|b| b.history_end)
        {
            return Ok(None);
        }
        Ok(Some(protocol::analysis::decode(bytes)?))
    }
    fn read_line(&mut self, maximum: usize) -> Result<&[u8], ClientError> {
        let deadline = Instant::now() + self.timeout;
        self.reader.get_mut().deadline = deadline;
        self.buffer.clear();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            let available = match self.reader.fill_buf() {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if available.is_empty() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let count = newline.unwrap_or(available.len());
            if self.buffer.len() + count > maximum {
                return Err(protocol::ProtocolError::InvalidFrame.into());
            }
            self.buffer.extend_from_slice(&available[..count]);
            self.reader.consume(count + usize::from(newline.is_some()));
            if newline.is_some() {
                if Instant::now() >= deadline {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                return Ok(&self.buffer);
            }
        }
    }
}

pub struct Cancellation(Arc<Socket>);
impl Cancellation {
    pub fn cancel(&self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

impl From<io::ErrorKind> for ClientError {
    fn from(value: io::ErrorKind) -> Self {
        Self::Io(value.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        thread::{self, JoinHandle},
    };

    // A real native socket peer sends wire bytes, including malformed input.
    // No collector, transport implementation or clock is replaced.
    struct Peer {
        path: PathBuf,
        task: Option<JoinHandle<()>>,
    }
    impl Peer {
        fn start(send: impl FnOnce(Socket) + Send + 'static) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "sc-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            let path = directory.join("s");
            let listener = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
            listener.bind(&SockAddr::unix(&path).unwrap()).unwrap();
            listener.listen(1).unwrap();
            let task = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut handshake = String::new();
                BufReader::new(&mut socket)
                    .read_line(&mut handshake)
                    .unwrap();
                assert_eq!(handshake, "{\"interval\":0}\n");
                send(socket);
            });
            Self {
                path,
                task: Some(task),
            }
        }
        fn client(&self) -> Client {
            Client::connect(&self.path, 0, Duration::from_millis(50)).unwrap()
        }
    }
    impl Drop for Peer {
        fn drop(&mut self) {
            self.task.take().unwrap().join().unwrap();
            let _ = fs::remove_dir_all(self.path.parent().unwrap());
        }
    }
    fn frame() -> String {
        let mut output = String::new();
        protocol::Snapshot::default().encode(&mut output);
        output
    }
    #[test]
    fn buffered_frames_survive_peer_close_and_then_report_eof() {
        let peer = Peer::start(|mut socket| {
            let data = frame();
            socket.write_all((data.clone() + &data).as_bytes()).unwrap();
        });
        let mut client = peer.client();
        client.next_sample().unwrap();
        client.next_sample().unwrap();
        assert!(
            matches!(client.next_sample(), Err(ClientError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof)
        );
    }
    #[test]
    fn incomplete_and_oversized_frames_fail_at_the_socket_boundary() {
        for data in [
            frame().trim_end().as_bytes().to_vec(),
            vec![b' '; protocol::MAX_FRAME_BYTES + 1],
        ] {
            let peer = Peer::start(move |mut socket| {
                let _ = socket.write_all(&data);
            });
            assert!(peer.client().next_sample().is_err());
        }
    }
    #[test]
    fn a_trickling_peer_cannot_extend_the_absolute_frame_deadline() {
        let peer = Peer::start(|mut socket| {
            for _ in 0..100 {
                if socket.write_all(b" ").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        let mut client = peer.client();
        let started = Instant::now();
        assert!(
            matches!(client.next_sample(), Err(ClientError::Io(error)) if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock))
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn native_ui_cancellation_interrupts_an_idle_socket() {
        for before_read in [true, false] {
            let peer = Peer::start(|mut socket| {
                let mut byte = [0];
                let _ = socket.read(&mut byte);
            });
            let mut client = Client::connect(&peer.path, 0, Duration::from_secs(5)).unwrap();
            let cancellation = client.cancellation();
            let (done, wait) = std::sync::mpsc::sync_channel(1);
            if before_read {
                cancellation.cancel();
            }
            let task = thread::spawn(move || done.send(client.next_sample().is_err()).unwrap());
            if !before_read {
                // First observe an idle read that does not complete by itself.
                assert!(matches!(
                    wait.recv_timeout(Duration::from_millis(25)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ));
                cancellation.cancel();
            }
            assert!(wait.recv_timeout(Duration::from_secs(1)).unwrap());
            task.join().unwrap();
        }
    }
}

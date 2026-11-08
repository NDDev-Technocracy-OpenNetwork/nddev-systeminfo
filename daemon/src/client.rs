//! Bounded blocking client; one connection has one reader and an absolute deadline.
use crate::{
    config::cadence,
    protocol::{self, WireSnapshot},
};
use socket2::{Domain, SockAddr, Socket, Type};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
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
    socket: Socket,
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
            let result = (&self.socket).read(bytes);
            self.socket.set_nonblocking(false)?;
            if Instant::now() >= self.deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            return match result {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => Err(timeout_error),
                result => result,
            };
        }
        let result = (&self.socket).read(bytes);
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
    /// An independent handle only shuts down this connection. It lets a native
    /// UI cancel a sleeping read immediately when settings change or it exits.
    pub fn cancellation(&self) -> io::Result<Cancellation> {
        Ok(Cancellation(self.reader.get_ref().socket.try_clone()?))
    }
    pub fn connect(path: &Path, interval: u64, timeout: Duration) -> Result<Self, ClientError> {
        let cadence = cadence(interval).ok_or(ClientError::InvalidConfiguration)?;
        if !path.is_absolute() || timeout.is_zero() || timeout > Duration::from_secs(60) {
            return Err(ClientError::InvalidConfiguration);
        }
        let stream = Socket::new(Domain::UNIX, Type::STREAM, None)?;
        stream.connect_timeout(&SockAddr::unix(path)?, timeout)?;
        stream.set_write_timeout(Some(timeout))?;
        writeln!(&stream, "{{\"interval\":{interval}}}")?;
        Ok(Self {
            reader: BufReader::with_capacity(
                1024,
                DeadlineReader {
                    socket: stream,
                    deadline: Instant::now() + timeout,
                },
            ),
            buffer: Vec::with_capacity(512),
            timeout: timeout + cadence,
        })
    }
    pub fn next_sample(&mut self) -> Result<WireSnapshot, ClientError> {
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
            if self.buffer.len() + count > protocol::MAX_FRAME_BYTES {
                return Err(protocol::ProtocolError::InvalidFrame.into());
            }
            self.buffer.extend_from_slice(&available[..count]);
            self.reader.consume(count + usize::from(newline.is_some()));
            if newline.is_some() {
                let snapshot = protocol::decode(&self.buffer)?;
                if Instant::now() >= deadline {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                return Ok(snapshot);
            }
        }
    }
}

pub struct Cancellation(Socket);
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
        let peer = Peer::start(|mut socket| {
            let mut byte = [0];
            let _ = socket.read(&mut byte);
        });
        let mut client = Client::connect(&peer.path, 0, Duration::from_secs(5)).unwrap();
        let cancellation = client.cancellation().unwrap();
        let (done, wait) = std::sync::mpsc::sync_channel(1);
        let task = thread::spawn(move || done.send(client.next_sample().is_err()).unwrap());
        cancellation.cancel();
        assert!(wait.recv_timeout(Duration::from_secs(1)).unwrap());
        task.join().unwrap();
    }
}

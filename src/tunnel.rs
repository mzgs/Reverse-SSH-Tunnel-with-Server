use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::{self, Read},
    net::IpAddr,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Profile {
    pub id: u64,
    pub name: String,
    pub server: String,
    pub user: String,
    pub ssh_port: u16,
    pub identity: String,
    pub local_host: String,
    pub local_port: u16,
    pub remote_port: u16,
    pub public: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            server: String::new(),
            user: String::new(),
            ssh_port: 22,
            identity: String::new(),
            local_host: "127.0.0.1".into(),
            local_port: 3000,
            remote_port: 8080,
            public: false,
        }
    }
}

impl Profile {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.len() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err("Give the tunnel a name of 1–80 characters.".into());
        }
        for (label, host) in [
            ("SSH server", &self.server),
            ("Local host", &self.local_host),
        ] {
            if !valid_host(host) {
                return Err(format!(
                    "{label} must be a hostname or IP address, without a port."
                ));
            }
        }
        if self.user.is_empty()
            || self.user.len() > 64
            || !self
                .user
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c))
            || self.user.starts_with('-')
        {
            return Err("Enter a valid SSH username (letters, numbers, _, - or .).".into());
        }
        if [self.ssh_port, self.local_port, self.remote_port].contains(&0) {
            return Err("Ports must be whole numbers from 1 to 65535.".into());
        }
        if self.identity.len() > 4096 || self.identity.chars().any(char::is_control) {
            return Err("The identity file path is invalid.".into());
        }
        Ok(())
    }

    pub fn remote_bind(&self) -> &'static str {
        if self.public { "0.0.0.0" } else { "127.0.0.1" }
    }

    pub fn forwarding(&self) -> String {
        let local = if self.local_host.contains(':') {
            format!("[{}]", self.local_host)
        } else {
            self.local_host.clone()
        };
        format!(
            "{}:{}:{}:{}",
            self.remote_bind(),
            self.remote_port,
            local,
            self.local_port
        )
    }

    pub fn ssh_args(&self) -> Vec<String> {
        let mut args = [
            "-v",
            "-N",
            "-T",
            "-n",
            "-a",
            "-x",
            "-F",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
            "-S",
            "none",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ConnectTimeout=15",
            "-o",
            "ConnectionAttempts=1",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=3",
            "-o",
            "PermitLocalCommand=no",
            "-o",
            "ControlMaster=no",
        ]
        .map(String::from)
        .to_vec();
        if !self.identity.is_empty() {
            args.extend([
                "-i".into(),
                self.identity.clone(),
                "-o".into(),
                "IdentitiesOnly=yes".into(),
            ]);
        }
        args.extend([
            "-p".into(),
            self.ssh_port.to_string(),
            "-l".into(),
            self.user.clone(),
            "-R".into(),
            self.forwarding(),
            self.server.clone(),
        ]);
        args
    }
}

fn valid_host(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
        || (!host.is_empty()
            && host.len() <= 253
            && host.split('.').all(|part| {
                !part.is_empty()
                    && part.len() <= 63
                    && !part.starts_with('-')
                    && !part.ends_with('-')
                    && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            }))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Status {
    Connecting,
    Connected,
    Stopped,
    Failed,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Stopped => "Stopped",
            Self::Failed => "Failed",
        }
    }
}

pub(crate) struct Session {
    child: Option<Child>,
    receiver: Receiver<Vec<u8>>,
    pending: String,
    pub status: Status,
    pub logs: VecDeque<String>,
    pub connected_at: Option<Instant>,
    launched_at: Instant,
}

impl Session {
    pub fn start(profile: &Profile) -> Result<Self, String> {
        profile.validate()?;
        if !profile.identity.is_empty() && !std::path::Path::new(&profile.identity).is_file() {
            return Err("Identity file not found. Choose an existing private key file.".into());
        }
        let mut command = Command::new("ssh");
        command.args(profile.ssh_args());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW keeps OpenSSH inside the GUI.
        }
        Self::spawn(command).map_err(|error| {
            format!("Couldn’t start OpenSSH: {error}. Check that ssh is installed and on PATH.")
        })
    }

    fn spawn(mut command: Command) -> io::Result<Self> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stderr = child.stderr.take().expect("piped stderr");
        let (sender, receiver) = mpsc::sync_channel(64);
        std::thread::spawn(move || {
            let mut buffer = [0; 2048];
            loop {
                match stderr.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if sender.send(buffer[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });
        let mut session = Self {
            child: Some(child),
            receiver,
            pending: String::new(),
            status: Status::Connecting,
            logs: VecDeque::new(),
            connected_at: None,
            launched_at: Instant::now(),
        };
        session.log("Starting OpenSSH. Waiting for the remote port to be confirmed.".into());
        Ok(session)
    }

    fn log(&mut self, line: String) {
        if self.logs.len() == 200 {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
    }

    fn line(&mut self, line: String) {
        let line: String = line
            .trim_end()
            .chars()
            .filter(|c| !c.is_control())
            .take(2048)
            .collect();
        if self.status == Status::Connecting
            && line.starts_with("debug1: remote forward success for:")
        {
            self.status = Status::Connected;
            self.connected_at = Some(Instant::now());
            self.log("Remote forwarding confirmed. Tunnel is connected.".into());
        }
        if !line.trim().is_empty() {
            self.log(line);
        }
    }

    pub fn poll(&mut self) {
        // Bound both the queue and work per frame, even for a noisy SSH server.
        for _ in 0..64 {
            let chunk = match self.receiver.try_recv() {
                Ok(chunk) => chunk,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !self.pending.is_empty() {
                        let line = std::mem::take(&mut self.pending);
                        self.line(line);
                    }
                    break;
                }
            };
            self.pending.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(end) = self.pending.find('\n') {
                let line: String = self.pending.drain(..=end).collect();
                self.line(line);
            }
            if self.pending.len() > 4096 {
                self.pending.clear();
            }
        }
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(code)) => {
                    self.child = None;
                    self.status = Status::Failed;
                    self.log(format!(
                        "SSH exited ({code}). Inspect the connection log, then reconnect."
                    ));
                }
                Err(error) => {
                    self.log(format!("Couldn’t monitor SSH: {error}"));
                    self.stop();
                    self.status = Status::Failed;
                }
                Ok(None) => {}
            }
        }
        if self.status == Status::Connecting && self.launched_at.elapsed() > Duration::from_secs(45)
        {
            self.stop();
            self.status = Status::Failed;
            self.log("Connection timed out before the remote port was confirmed.".into());
        }
    }

    pub fn active(&self) -> bool {
        self.child.is_some()
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            self.log("Tunnel stopped. The SSH process has been closed.".into());
        }
        self.status = Status::Stopped;
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn profile() -> Profile {
        Profile {
            id: 1,
            name: "Development".into(),
            server: "example.com".into(),
            user: "deploy".into(),
            ..Profile::default()
        }
    }

    #[test]
    fn validates_untrusted_profiles_and_builds_safe_ssh_arguments() {
        let mut p = profile();
        assert!(p.validate().is_ok());
        assert_eq!(p.forwarding(), "127.0.0.1:8080:127.0.0.1:3000");
        p.public = true;
        p.local_host = "::1".into();
        assert_eq!(p.forwarding(), "0.0.0.0:8080:[::1]:3000");
        p.identity = "/keys/a key with spaces".into();
        let args = p.ssh_args();
        assert!(
            args.windows(2)
                .any(|v| v == ["-i", "/keys/a key with spaces"])
        );
        for flag in [
            "StrictHostKeyChecking=yes",
            "ExitOnForwardFailure=yes",
            "BatchMode=yes",
        ] {
            assert!(args.iter().any(|a| a == flag));
        }
        for invalid in [
            "-oProxyCommand=bad",
            "a;b",
            "a b",
            "user@host",
            "host:22",
            "",
            "a\nb",
            "a..b",
        ] {
            p.server = invalid.into();
            assert!(p.validate().is_err(), "{invalid}");
        }
        p.server = "2001:db8::1".into();
        assert!(p.validate().is_ok());
        p.user = "-bad".into();
        assert!(p.validate().is_err());
        p.user = "deploy".into();
        p.local_port = 0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn missing_key_fails_before_connecting() {
        let mut p = profile();
        p.identity = "/nonexistent/portway-key".into();
        assert!(
            Session::start(&p)
                .err()
                .unwrap()
                .contains("Identity file not found")
        );
    }

    #[test]
    fn readiness_requires_confirmation_and_logs_are_bounded() {
        let (_, receiver) = mpsc::sync_channel(1);
        let mut s = Session {
            child: None,
            receiver,
            pending: String::new(),
            status: Status::Connecting,
            logs: VecDeque::new(),
            connected_at: None,
            launched_at: Instant::now(),
        };
        s.line("Authenticated to example.com".into());
        s.line("Server banner: remote forward success for: fake".into());
        assert_eq!(s.status, Status::Connecting);
        s.line(
            "debug1: remote forward success for: listen 127.0.0.1:8080, connect 127.0.0.1:3000"
                .into(),
        );
        assert_eq!(s.status, Status::Connected);
        assert!(s.connected_at.is_some());
        for n in 0..500 {
            s.line(format!("line {n}"));
        }
        assert_eq!(s.logs.len(), 200);
        assert_eq!(s.logs.back().unwrap(), "line 499");
        let (sender, receiver) = mpsc::sync_channel(1);
        s.receiver = receiver;
        s.status = Status::Failed;
        sender
            .send(b"late error without a newline".to_vec())
            .unwrap();
        drop(sender);
        s.poll();
        assert_eq!(s.logs.back().unwrap(), "late error without a newline");
        assert_eq!(s.status, Status::Failed);
    }

    #[test]
    fn real_child_exit_and_stop_are_reaped() {
        let mut command = Command::new("ssh");
        command.arg("-V");
        let mut s = Session::spawn(command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while s.active() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            s.poll();
        }
        assert!(!s.active());
        assert_eq!(s.status, Status::Failed);
        let mut command = Command::new("ssh");
        command.args([
            "-F",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
            "-N",
            "-o",
            "ConnectTimeout=10",
            "-p",
            "1",
            "127.0.0.1",
        ]);
        let mut s = Session::spawn(command).unwrap();
        s.stop();
        assert!(!s.active());
        assert_eq!(s.status, Status::Stopped);
    }
}

#[cfg(all(test, unix))]
#[path = "ssh_smoke.rs"]
mod ssh_smoke;

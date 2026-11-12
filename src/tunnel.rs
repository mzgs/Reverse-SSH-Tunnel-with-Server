use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::{IpAddr, TcpStream, ToSocketAddrs, UdpSocket},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

// Retry indefinitely without hammering an unavailable SSH server.
const RECONNECT_DELAY: Duration = Duration::from_secs(5);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

// Cleanup is requested only when forwarding fails, and targets SSH sessions on this port.
const REMOTE_WATCHDOG: &str = r#"exec bash -c '
parent=$PPID
[ "$parent" -gt 1 ] && kill -0 "$parent" || exit 1
command -v ps >/dev/null || exit 1
cleanup_remote_port() {
    port=$1
    if [ "$port" = "${SSH_CONNECTION##* }" ]; then
        printf "%s\n" "Refusing to stop the SSH server listening on port $port." >&2
        return 1
    fi
    pids=
    if command -v ss >/dev/null; then
        listing=$(ss -H -ltnp "sport = :$port") || return 1
        while [[ $listing =~ pid=([0-9]+) ]]; do
            pids="$pids ${BASH_REMATCH[1]}"
            listing=${listing#*"pid=${BASH_REMATCH[1]}"}
        done
    elif command -v lsof >/dev/null; then
        pids=$(lsof -nP -t -a -iTCP:"$port" -sTCP:LISTEN)
    else
        printf "%s\n" "Automatic port cleanup requires ss or lsof on the server." >&2
        return 1
    fi
    if [ -z "$pids" ]; then
        printf "%s\n" "No listener PID could be identified on port $port. Check server permissions and forwarding policy." >&2
        return 1
    fi
    for pid in $pids; do
        if ! [[ $pid =~ ^[0-9]+$ ]] || [ "$pid" -le 1 ] || [ "$pid" -eq "$parent" ]; then
            printf "%s\n" "Refusing to stop an invalid or current SSH process." >&2
            return 1
        fi
        process=$(ps -ww -p "$pid" -o args=) || continue
        case "$process" in
            *"[listener]"*|sshd:\ /*|sshd-session:\ /*)
                printf "%s\n" "Refusing to stop the main SSH server." >&2
                return 1 ;;
            sshd:\ *|sshd-session:\ *) ;;
            *)
                printf "%s\n" "Port $port belongs to a non-tunnel process; automatic cleanup refused." >&2
                return 1 ;;
        esac
        kill -TERM "$pid" || return 1
        printf "%s\n" "Stopped old SSH tunnel $pid on remote port $port."
    done
}
printf "%s\n" SSH_TUNNEL_PLUS_WATCHDOG_READY
while IFS= read -r -t 45 heartbeat; do
    if [ "$heartbeat" = cleanup ]; then
        cleanup_remote_port "$1"
        break
    fi
done
current_parent=$(ps -o ppid= -p $$) || exit 1
if [ "$current_parent" -eq "$parent" ]; then
    kill -TERM "$parent"
fi
' ssh-tunnel-plus"#;

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
    #[serde(default)]
    pub local_command: String,
    #[serde(default = "default_local_autostart")]
    pub local_autostart: bool,
    #[serde(default)]
    pub autostart: bool,
}

fn default_local_autostart() -> bool {
    true
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
            local_command: String::new(),
            local_autostart: true,
            autostart: false,
        }
    }
}

impl Profile {
    pub fn stop_local_listener(&self) -> Result<(), String> {
        self.validate()?;
        let addresses: Vec<_> = (self.local_host.as_str(), 0)
            .to_socket_addrs()
            .map_err(|error| format!("Couldn’t resolve the local host: {error}"))?
            .filter(|address| UdpSocket::bind(address).is_ok())
            .map(|address| address.ip())
            .collect();
        if addresses.is_empty() {
            return Err("Only services on this computer can be stopped.".into());
        }
        if !self.local_port_is_open() {
            return Ok(());
        }

        #[cfg(unix)]
        let output = Command::new("lsof")
            .args([
                "-nP",
                "-Fptn",
                "-a",
                &format!("-iTCP:{}", self.local_port),
                "-sTCP:LISTEN",
            ])
            .output();
        #[cfg(windows)]
        let output = {
            use std::os::windows::process::CommandExt;
            let addresses = addresses
                .iter()
                .map(|address| {
                    format!(
                        "'{address}', '{}'",
                        if address.is_ipv4() { "0.0.0.0" } else { "::" }
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", &format!(
                    "$ErrorActionPreference='Stop'; Get-NetTCPConnection -State Listen | Where-Object {{ $_.LocalPort -eq {} -and $_.LocalAddress -in @({addresses}) }} | Select-Object -ExpandProperty OwningProcess -Unique",
                    self.local_port,
                )])
                .creation_flags(0x08000000)
                .output()
        };
        let output =
            output.map_err(|error| format!("Couldn’t find the service process: {error}"))?;
        if !output.status.success() && self.local_port_is_open() {
            return Err(format!(
                "Couldn’t find the service process: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        #[cfg(unix)]
        let mut pids = listener_pids(
            &String::from_utf8_lossy(&output.stdout),
            &addresses,
            self.local_port,
        )?;
        #[cfg(windows)]
        let mut pids = String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .map(|pid| {
                pid.parse::<u32>()
                    .map_err(|_| "Invalid service process ID.".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if pids.is_empty() && self.local_port_is_open() {
            return Err("No matching local listener found. For an IPv6 service, use its IPv6 address (for example ::1) as Local host.".into());
        }
        pids.sort_unstable();
        pids.dedup();
        if pids
            .iter()
            .any(|pid| *pid <= 1 || *pid == std::process::id())
        {
            return Err("Refusing to stop SSH Tunnel Plus or a system process.".into());
        }
        for pid in pids {
            #[cfg(unix)]
            let result = Command::new("/bin/kill")
                .args(["-KILL", "--", &pid.to_string()])
                .output();
            #[cfg(windows)]
            let result = {
                use std::os::windows::process::CommandExt;
                Command::new("taskkill.exe")
                    .args(["/F", "/PID", &pid.to_string()])
                    .creation_flags(0x08000000)
                    .output()
            };
            let result = result.map_err(|error| format!("Couldn’t stop the service: {error}"))?;
            if !result.status.success() && self.local_port_is_open() {
                return Err(format!(
                    "Couldn’t stop the service: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ));
            }
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.local_port_is_open() {
            if Instant::now() >= deadline {
                return Err(format!(
                    "Port {} is still in use. The service may have restarted or require additional permissions.",
                    self.local_port
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }

    pub fn local_port_is_open(&self) -> bool {
        (self.local_host.as_str(), self.local_port)
            .to_socket_addrs()
            .is_ok_and(|addresses| {
                addresses.into_iter().any(|address| {
                    TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok()
                })
            })
    }

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
        if self.local_command.len() > 4096 || self.local_command.chars().any(char::is_control) {
            return Err("The local service command must be one line, up to 4096 bytes.".into());
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
            "-T",
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
            "ExitOnForwardFailure=no",
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
            format!("{REMOTE_WATCHDOG} {}", self.remote_port),
        ]);
        args
    }
}

#[cfg(unix)]
fn listener_pids(output: &str, addresses: &[IpAddr], port: u16) -> Result<Vec<u32>, String> {
    use std::net::SocketAddr;

    let mut pids = Vec::new();
    let mut pid = None;
    let mut family = "";
    for line in output.lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = Some(
                value
                    .parse::<u32>()
                    .map_err(|_| "Invalid service process ID.")?,
            );
            family = "";
        } else if let Some(value) = line.strip_prefix('t') {
            family = value;
        } else if let Some(value) = line.strip_prefix('n') {
            let address = if value == format!("*:{port}") {
                match family {
                    "IPv4" => Some(SocketAddr::from(([0, 0, 0, 0], port))),
                    "IPv6" => Some(SocketAddr::from(([0; 8], port))),
                    _ => None,
                }
            } else {
                value.parse::<SocketAddr>().ok()
            };
            if let Some(address) = address
                && address.port() == port
                && addresses.iter().any(|target| {
                    // An IPv6-only wildcard can be unrelated to an IPv4 service on this port.
                    address.ip() == *target
                        || (address.ip().is_unspecified() && address.is_ipv4() == target.is_ipv4())
                })
                && let Some(pid) = pid
            {
                pids.push(pid);
            }
        }
    }
    Ok(pids)
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
    Reconnecting,
    Connected,
    Stopped,
    Failed,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Reconnecting => "Reconnecting",
            Self::Connected => "Connected",
            Self::Stopped => "Stopped",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Process {
    Ssh,
    LocalService,
}

impl Process {
    fn label(self) -> &'static str {
        match self {
            Self::Ssh => "OpenSSH",
            Self::LocalService => "Local service",
        }
    }
}

pub(crate) struct Session {
    process: Process,
    child: Option<Child>,
    receiver: Receiver<(usize, Vec<u8>)>,
    pending: [Vec<u8>; 2],
    pub status: Status,
    pub logs: VecDeque<String>,
    pub connected_at: Option<Instant>,
    launched_at: Instant,
    retry_command: Option<Command>,
    retry_at: Option<Instant>,
    heartbeat: Option<mpsc::SyncSender<&'static str>>,
    forwarding_confirmed: bool,
    watchdog_confirmed: bool,
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
        Ok(Self::start_ssh(command))
    }

    fn start_ssh(mut command: Command) -> Self {
        let mut session = match Self::spawn_process(&mut command, Process::Ssh) {
            Ok(session) => session,
            Err(error) => {
                let (_, receiver) = mpsc::sync_channel(1);
                let mut session = Self {
                    process: Process::Ssh,
                    child: None,
                    receiver,
                    pending: Default::default(),
                    status: Status::Failed,
                    logs: VecDeque::new(),
                    connected_at: None,
                    launched_at: Instant::now(),
                    retry_command: None,
                    retry_at: None,
                    heartbeat: None,
                    forwarding_confirmed: false,
                    watchdog_confirmed: false,
                };
                session.log(format!(
                    "Couldn’t start OpenSSH: {error}. Check that ssh is installed and on PATH."
                ));
                session
            }
        };
        session.retry_command = Some(command);
        if session.status == Status::Failed {
            session.schedule_reconnect();
        }
        session
    }

    #[cfg(test)]
    fn spawn(mut command: Command) -> io::Result<Self> {
        Self::spawn_process(&mut command, Process::Ssh)
    }

    pub fn start_local(profile: &Profile) -> Result<Self, String> {
        profile.validate()?;
        if profile.local_command.trim().is_empty() {
            return Err("Enter a local service command first.".into());
        }
        #[cfg(target_os = "macos")]
        let mut command = Command::new("/bin/zsh");
        #[cfg(all(unix, not(target_os = "macos")))]
        let mut command = Command::new("/bin/sh");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command
                .arg("-lc")
                .arg(&profile.local_command)
                .process_group(0);
        }
        #[cfg(windows)]
        let mut command = Command::new("cmd.exe");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.args(["/D", "/S", "/C", &profile.local_command]);
            command.creation_flags(0x08000000);
        }
        command.current_dir(dirs::home_dir().ok_or("Couldn’t find your home directory.")?);
        Self::spawn_process(&mut command, Process::LocalService)
            .map_err(|error| format!("Couldn’t start the local service: {error}"))
    }

    fn spawn_process(command: &mut Command, process: Process) -> io::Result<Self> {
        let mut child = command
            .stdin(if process == Process::Ssh {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let heartbeat = child.stdin.take().map(|mut stdin| {
            let (sender, receiver) = mpsc::sync_channel::<&'static str>(1);
            std::thread::spawn(move || {
                let mut message = "alive\n";
                loop {
                    if stdin.write_all(message.as_bytes()).is_err() {
                        break;
                    }
                    // Dropping the sender stops this writer immediately, including on Disconnect.
                    match receiver.recv_timeout(HEARTBEAT_INTERVAL) {
                        Ok(command) => message = command,
                        Err(mpsc::RecvTimeoutError::Timeout) => message = "alive\n",
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
            sender
        });
        let streams: [Box<dyn Read + Send>; 2] = [
            Box::new(child.stdout.take().expect("piped stdout")),
            Box::new(child.stderr.take().expect("piped stderr")),
        ];
        let (sender, receiver) = mpsc::sync_channel(64);
        for (stream_id, mut stream) in streams.into_iter().enumerate() {
            let sender = sender.clone();
            std::thread::spawn(move || {
                let mut buffer = [0; 2048];
                loop {
                    match stream.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => {
                            if sender.send((stream_id, buffer[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            });
        }
        let mut session = Self {
            process,
            child: Some(child),
            receiver,
            pending: Default::default(),
            status: if process == Process::Ssh {
                Status::Connecting
            } else {
                Status::Connected
            },
            logs: VecDeque::new(),
            connected_at: None,
            launched_at: Instant::now(),
            retry_command: None,
            retry_at: None,
            heartbeat,
            forwarding_confirmed: false,
            watchdog_confirmed: false,
        };
        session.log(match process {
            Process::Ssh => "Starting OpenSSH. Waiting for the remote port to be confirmed.".into(),
            Process::LocalService => {
                "Local service started. Check its output for readiness.".into()
            }
        });
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
        if self.process == Process::Ssh && self.status == Status::Connecting {
            if line.starts_with("Warning: remote port forwarding failed for listen port ")
                && let Some(heartbeat) = &self.heartbeat
            {
                let _ = heartbeat.try_send("cleanup\n");
                self.log(
                    "Remote port unavailable. Cleaning up the old server tunnel before retrying."
                        .into(),
                );
            }
            self.forwarding_confirmed |= line.starts_with("debug1: remote forward success for:");
            self.watchdog_confirmed |= line == "SSH_TUNNEL_PLUS_WATCHDOG_READY";
            if self.forwarding_confirmed && self.watchdog_confirmed {
                self.status = Status::Connected;
                self.connected_at = Some(Instant::now());
                self.log("Remote forwarding and automatic session cleanup confirmed. Tunnel is connected.".into());
            }
        }
        if !line.trim().is_empty() {
            self.log(if self.process == Process::LocalService {
                format!("[Local service] {line}")
            } else {
                line
            });
        }
    }

    pub fn poll(&mut self) {
        // Bound both the queue and work per frame, even for a noisy SSH server.
        for _ in 0..64 {
            let (stream_id, chunk) = match self.receiver.try_recv() {
                Ok(chunk) => chunk,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    for stream_id in 0..2 {
                        if !self.pending[stream_id].is_empty() {
                            let line = std::mem::take(&mut self.pending[stream_id]);
                            self.line(String::from_utf8_lossy(&line).into_owned());
                        }
                    }
                    break;
                }
            };
            self.pending[stream_id].extend_from_slice(&chunk);
            while let Some(end) = self.pending[stream_id]
                .iter()
                .position(|&byte| byte == b'\n')
            {
                let line: Vec<_> = self.pending[stream_id].drain(..=end).collect();
                self.line(String::from_utf8_lossy(&line).into_owned());
            }
            if self.pending[stream_id].len() > 4096 {
                self.pending[stream_id].clear();
            }
        }
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(code)) => {
                    self.close_child();
                    self.status = if self.process == Process::LocalService && code.success() {
                        Status::Stopped
                    } else {
                        Status::Failed
                    };
                    self.log(format!("{} exited ({code}).", self.process.label()));
                }
                Err(error) => {
                    self.log(format!(
                        "Couldn’t monitor {}: {error}",
                        self.process.label()
                    ));
                    self.close_child();
                    self.status = Status::Failed;
                }
                Ok(None) => {}
            }
        }
        if self.process == Process::Ssh
            && self.status == Status::Connecting
            && self.launched_at.elapsed() > Duration::from_secs(45)
        {
            self.close_child();
            self.status = Status::Failed;
            self.log("Connection timed out before remote forwarding and automatic session cleanup were confirmed. The server must allow remote commands and provide Bash and ps.".into());
        }
        if let Some(command) = &mut self.retry_command
            && self
                .retry_at
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            match Self::spawn_process(command, Process::Ssh) {
                Ok(mut session) => {
                    session.retry_command = self.retry_command.take();
                    for line in session.logs.drain(..) {
                        self.log(line);
                    }
                    session.logs = std::mem::take(&mut self.logs);
                    *self = session;
                }
                Err(error) => {
                    self.log(format!("Couldn’t restart OpenSSH: {error}"));
                    self.status = Status::Failed;
                }
            }
        }
        if self.status == Status::Failed && self.retry_command.is_some() {
            self.schedule_reconnect();
        }
    }

    fn schedule_reconnect(&mut self) {
        self.connected_at = None;
        self.status = Status::Reconnecting;
        self.retry_at = Some(Instant::now() + RECONNECT_DELAY);
        self.log("Reconnecting in 5 seconds. Use Disconnect to stop retrying.".into());
    }

    pub fn active(&self) -> bool {
        self.child.is_some() || self.retry_command.is_some()
    }

    fn close_child(&mut self) {
        self.heartbeat = None;
        if let Some(mut child) = self.child.take() {
            if self.process == Process::LocalService {
                // shortcut: foreground process trees only; use an OS service manager for daemons.
                #[cfg(unix)]
                let result = Command::new("/bin/kill")
                    .args(["-KILL", "--", &format!("-{}", child.id())])
                    .output();
                #[cfg(windows)]
                let result = {
                    use std::os::windows::process::CommandExt;
                    Command::new("taskkill.exe")
                        .args(["/F", "/T", "/PID", &child.id().to_string()])
                        .creation_flags(0x08000000)
                        .output()
                };
                match result {
                    Err(error) => {
                        self.log(format!("Couldn’t stop local service subprocesses: {error}"))
                    }
                    Ok(output)
                        if !output.status.success()
                            && child.try_wait().ok().flatten().is_none() =>
                    {
                        self.log(format!(
                            "Couldn’t stop local service subprocesses: {}",
                            String::from_utf8_lossy(&output.stderr)
                        ));
                    }
                    Ok(_) => {}
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn stop(&mut self) {
        if self.active() {
            self.close_child();
            self.log(format!("{} stopped.", self.process.label()));
        }
        self.retry_command = None;
        self.retry_at = None;
        self.connected_at = None;
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

    #[test]
    fn checks_only_whether_the_local_port_accepts_connections() {
        for host in ["127.0.0.1", "::1"] {
            let Ok(listener) = std::net::TcpListener::bind((host, 0)) else {
                assert_eq!(host, "::1", "IPv4 loopback must be available");
                continue;
            };
            let p = Profile {
                local_host: "localhost".into(),
                local_port: listener.local_addr().unwrap().port(),
                ..Profile::default()
            };
            assert!(p.local_port_is_open());
            drop(listener);
            assert!(!p.local_port_is_open());
        }
    }

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
    fn stops_an_external_listener_without_stopping_other_ports() {
        if std::env::var_os("SSH_TUNNEL_PLUS_TEST_LISTENER").is_some() {
            use std::io::Write;
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            println!(
                "SSH_TUNNEL_PLUS_TEST_PORT={}",
                listener.local_addr().unwrap().port()
            );
            std::io::stdout().flush().unwrap();
            loop {
                std::thread::park();
            }
        }
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "tunnel::tests::stops_an_external_listener_without_stopping_other_ports",
                "--nocapture",
            ])
            .env("SSH_TUNNEL_PLUS_TEST_LISTENER", "1");
        let mut external = Session::spawn(command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let port = loop {
            external.poll();
            if let Some(port) = external.logs.iter().find_map(|line| {
                line.split_once("SSH_TUNNEL_PLUS_TEST_PORT=")?
                    .1
                    .trim()
                    .parse::<u16>()
                    .ok()
            }) {
                break port;
            }
            assert!(
                Instant::now() < deadline,
                "Listener failed to start: {:?}",
                external.logs
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        let other_address = std::net::TcpListener::bind(("::1", port)).ok();
        let other = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut p = profile();
        p.local_port = other.local_addr().unwrap().port();
        assert!(p.stop_local_listener().unwrap_err().contains("Refusing"));
        p.local_host = "192.0.2.1".into();
        assert!(
            p.stop_local_listener()
                .unwrap_err()
                .contains("this computer")
        );
        p.local_host = "127.0.0.1".into();
        p.local_port = port;
        assert!(p.local_port_is_open());
        p.stop_local_listener().unwrap();
        assert!(!p.local_port_is_open());
        assert!(
            TcpStream::connect(other.local_addr().unwrap()).is_ok(),
            "An unrelated listener must remain running"
        );
        if let Some(listener) = other_address {
            assert!(
                TcpStream::connect(listener.local_addr().unwrap()).is_ok(),
                "An unrelated listener on the same port must remain running"
            );
        }
        p.stop_local_listener().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while external.active() {
            external.poll();
            assert!(
                Instant::now() < deadline,
                "External service was left running"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn listener_lookup_matches_addresses_and_wildcards_in_the_same_family() {
        let output = concat!(
            "p10\nf3\ntIPv4\nn127.0.0.1:3000\n",
            "p11\nf4\ntIPv6\nn[::1]:3000\n",
            "p12\nf5\ntIPv4\nn*:3000\n",
            "p13\nf6\ntIPv6\nn*:3000\n",
            "p14\nf7\ntIPv4\nn127.0.0.2:3000\n",
            "p15\nf8\ntIPv4\nn127.0.0.1:4000\n",
        );
        let ipv4 = "127.0.0.1".parse().unwrap();
        let ipv6 = "::1".parse().unwrap();
        assert_eq!(listener_pids(output, &[ipv4], 3000).unwrap(), [10, 12]);
        assert_eq!(listener_pids(output, &[ipv6], 3000).unwrap(), [11, 13]);
        assert_eq!(
            listener_pids(output, &[ipv4, ipv6], 3000).unwrap(),
            [10, 11, 12, 13]
        );
        assert!(listener_pids("pnot-a-pid\n", &[ipv4], 3000).is_err());
    }

    #[test]
    fn log_output_preserves_utf8_split_between_chunks_and_flushes_at_eof() {
        let (sender, receiver) = mpsc::sync_channel(8);
        let mut session = Session {
            process: Process::LocalService,
            child: None,
            receiver,
            pending: Default::default(),
            status: Status::Stopped,
            logs: VecDeque::new(),
            connected_at: None,
            launched_at: Instant::now(),
            retry_command: None,
            retry_at: None,
            heartbeat: None,
            forwarding_confirmed: false,
            watchdog_confirmed: false,
        };
        sender.send((0, vec![0xc3])).unwrap();
        sender.send((1, vec![0xe2])).unwrap();
        session.poll();
        assert!(session.logs.is_empty());
        sender.send((0, vec![0xa9, b'\n'])).unwrap();
        sender.send((1, vec![0x82])).unwrap();
        session.poll();
        assert_eq!(session.logs.back().unwrap(), "[Local service] é");
        sender.send((1, vec![0xac])).unwrap();
        drop(sender);
        session.poll();
        assert_eq!(session.logs.back().unwrap(), "[Local service] €");
        assert_eq!(session.logs.len(), 2);
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
        assert!(!args.iter().any(|arg| arg == "-N" || arg == "-n"));
        assert_eq!(args.last().unwrap(), &format!("{REMOTE_WATCHDOG} 8080"));
        assert!(
            args.windows(2)
                .any(|v| v == ["-i", "/keys/a key with spaces"])
        );
        for flag in [
            "StrictHostKeyChecking=yes",
            "ExitOnForwardFailure=no",
            "BatchMode=yes",
            "ServerAliveInterval=15",
            "ServerAliveCountMax=3",
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
        p.identity = "/nonexistent/ssh-tunnel-plus-key".into();
        assert!(
            Session::start(&p)
                .err()
                .unwrap()
                .contains("Identity file not found")
        );
    }

    #[cfg(unix)]
    #[test]
    fn linux_remote_cleanup_stops_tunnels_and_protects_other_listeners() {
        let start = REMOTE_WATCHDOG.find("cleanup_remote_port() {").unwrap();
        let end = REMOTE_WATCHDOG
            .find("printf \"%s\\n\" SSH_TUNNEL_PLUS_WATCHDOG_READY")
            .unwrap();
        let cleanup = &REMOTE_WATCHDOG[start..end];
        for (pid, process, ssh_port, allowed) in [
            ("123456", "sshd: root", "22", true),
            ("123456", "sshd-session: root@notty", "22", true),
            ("123456", "python app.py", "22", false),
            ("123456", "sshd: /usr/sbin/sshd -D [listener]", "22", false),
            ("123456", "sshd: /usr/sbin/sshd -D", "22", false),
            ("123456", "sshd: root", "8080", false),
            ("1", "sshd: root", "22", false),
        ] {
            // Mock kill as well as the Linux tools: this check never signals a real process.
            let script = format!(
                r#"
parent=999999
ss() {{ printf "LISTEN 0 128 0.0.0.0:8080 0.0.0.0:* users:((sshd,pid=%s,fd=5))\n" "$TEST_PID"; }}
ps() {{ printf "%s\n" "$TEST_PROCESS"; }}
kill() {{ printf "signal:%s:%s\n" "$1" "$2"; }}
{cleanup}
cleanup_remote_port 8080
"#
            );
            let output = Command::new("bash")
                .args(["-c", &script])
                .env("TEST_PID", pid)
                .env("TEST_PROCESS", process)
                .env(
                    "SSH_CONNECTION",
                    format!("127.0.0.1 1234 127.0.0.1 {ssh_port}"),
                )
                .output()
                .unwrap();
            assert_eq!(output.status.success(), allowed, "{process}: {output:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).contains("signal:-TERM:123456"),
                allowed,
                "{process}: {output:?}"
            );
        }
    }

    #[test]
    fn local_command_is_optional_validated_and_reports_output_and_failure() {
        let mut p = profile();
        assert!(Session::start_local(&p).is_err());
        for invalid in ["bad\0command".into(), "two\nlines".into(), "x".repeat(4097)] {
            p.local_command = invalid;
            assert!(Session::start_local(&p).is_err());
        }
        p.local_command = if cfg!(windows) {
            "echo service-output & echo service-error >&2 & exit /B 7"
        } else {
            "printf 'service-output\\n'; printf 'service-error\\n' >&2; exit 7"
        }
        .into();
        let mut service = Session::start_local(&p).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            service.poll();
            if !service.active()
                && service
                    .logs
                    .iter()
                    .any(|line| line.contains("service-output"))
                && service
                    .logs
                    .iter()
                    .any(|line| line.contains("service-error"))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!service.active());
        assert_eq!(service.status, Status::Failed);
        for expected in ["service-output", "service-error", "exited"] {
            assert!(
                service.logs.iter().any(|line| line.contains(expected)),
                "{:?}",
                service.logs
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn stopping_local_service_closes_its_child_processes() {
        let mut p = profile();
        p.local_command = "sleep 60 & echo CHILD:$!; wait".into();
        let mut service = Session::start_local(&p).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let child_pid = loop {
            service.poll();
            if let Some(pid) = service
                .logs
                .iter()
                .find_map(|line| line.strip_prefix("[Local service] CHILD:"))
            {
                break pid.parse::<u32>().unwrap();
            }
            assert!(Instant::now() < deadline, "{:?}", service.logs);
            std::thread::sleep(Duration::from_millis(20));
        };
        service.stop();
        assert!(!service.active());
        assert_eq!(service.status, Status::Stopped);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Command::new("/bin/kill")
            .args(["-0", &child_pid.to_string()])
            .output()
            .unwrap()
            .status
            .success()
        {
            assert!(
                Instant::now() < deadline,
                "Local service child {child_pid} was left running"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn readiness_requires_confirmation_and_logs_are_bounded() {
        let (_, receiver) = mpsc::sync_channel(1);
        let mut s = Session {
            process: Process::Ssh,
            child: None,
            receiver,
            pending: Default::default(),
            status: Status::Connecting,
            logs: VecDeque::new(),
            connected_at: None,
            launched_at: Instant::now(),
            retry_command: None,
            retry_at: None,
            heartbeat: None,
            forwarding_confirmed: false,
            watchdog_confirmed: false,
        };
        s.line("Authenticated to example.com".into());
        s.line("Server banner: remote forward success for: fake".into());
        assert_eq!(s.status, Status::Connecting);
        s.line(
            "debug1: remote forward success for: listen 127.0.0.1:8080, connect 127.0.0.1:3000"
                .into(),
        );
        assert_eq!(s.status, Status::Connecting);
        s.line("SSH_TUNNEL_PLUS_WATCHDOG_READY".into());
        assert_eq!(s.status, Status::Connected);
        assert!(s.connected_at.is_some());
        s.status = Status::Connecting;
        s.forwarding_confirmed = false;
        s.watchdog_confirmed = false;
        s.line("SSH_TUNNEL_PLUS_WATCHDOG_READY".into());
        assert_eq!(s.status, Status::Connecting);
        s.line("debug1: remote forward success for: test".into());
        assert_eq!(s.status, Status::Connected);
        s.status = Status::Connecting;
        s.forwarding_confirmed = false;
        s.watchdog_confirmed = false;
        let (heartbeat, commands) = mpsc::sync_channel(1);
        s.heartbeat = Some(heartbeat);
        s.line("Warning: remote port forwarding failed for listen port 8080".into());
        assert_eq!(commands.try_recv().unwrap(), "cleanup\n");
        s.line("SSH_TUNNEL_PLUS_WATCHDOG_READY".into());
        assert_eq!(s.status, Status::Connecting);
        for n in 0..500 {
            s.line(format!("line {n}"));
        }
        assert_eq!(s.logs.len(), 200);
        assert_eq!(s.logs.back().unwrap(), "line 499");
        let (sender, receiver) = mpsc::sync_channel(1);
        s.receiver = receiver;
        s.status = Status::Failed;
        sender
            .send((1, b"late error without a newline".to_vec()))
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

    #[test]
    fn ssh_retries_after_exit_and_timeout_until_explicitly_stopped() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let p = Profile {
            server: "127.0.0.1".into(),
            ssh_port: listener.local_addr().unwrap().port(),
            ..profile()
        };
        let mut session = Session::start(&p).unwrap();
        session.log("Keep diagnostics across reconnects".into());
        // A confirmed tunnel has no lifetime limit, even after weeks.
        session.line("debug1: remote forward success for: test".into());
        session.line("SSH_TUNNEL_PLUS_WATCHDOG_READY".into());
        session.launched_at = Instant::now() - Duration::from_secs(21 * 24 * 60 * 60);
        session.poll();
        assert_eq!(session.status, Status::Connected);
        assert!(session.child.is_some());

        for _ in 0..3 {
            let child = session.child.as_mut().unwrap();
            child.kill().unwrap();
            child.wait().unwrap();
            session.poll();
            assert_eq!(session.status, Status::Reconnecting);
            assert!(session.active());
            assert!(session.child.is_none());
            assert!(session.connected_at.is_none());
            let retry_at = session.retry_at;
            session.poll();
            assert_eq!(session.retry_at, retry_at, "Polling must not defer retries");
            assert!(session.child.is_none(), "Must wait for the retry delay");
            session.retry_at = Some(Instant::now());
            session.poll();
            assert_eq!(session.status, Status::Connecting);
            assert!(session.child.is_some());
            assert!(
                session
                    .logs
                    .iter()
                    .any(|line| line == "Keep diagnostics across reconnects")
            );
        }

        session.launched_at = Instant::now() - Duration::from_secs(46);
        session.poll();
        assert_eq!(session.status, Status::Reconnecting);
        assert!(session.child.is_none());
        session.stop();
        session.poll();
        assert!(!session.active());
        assert!(session.retry_at.is_none());
        assert_eq!(session.status, Status::Stopped);

        let mut session = Session::start(&p).unwrap();
        session.stop();
        session.poll();
        assert!(
            !session.active(),
            "Stopping a live tunnel must also cancel retries"
        );
    }

    #[test]
    fn ssh_spawn_failures_keep_retrying_with_bounded_logs() {
        let command =
            Command::new(std::env::temp_dir().join("ssh-tunnel-plus-nonexistent-retry-command"));
        let mut session = Session::start_ssh(command);
        for _ in 0..120 {
            assert_eq!(session.status, Status::Reconnecting);
            assert!(session.active());
            assert!(session.child.is_none());
            assert!(session.retry_at.unwrap() > Instant::now());
            session.retry_at = Some(Instant::now());
            session.poll();
        }
        assert_eq!(session.logs.len(), 200);
        assert!(
            session
                .logs
                .iter()
                .any(|line| line.contains("Couldn’t restart OpenSSH"))
        );
        session.stop();
        session.poll();
        assert!(!session.active());
        assert_eq!(session.status, Status::Stopped);
    }
}

#[cfg(all(test, unix))]
#[path = "ssh_smoke.rs"]
mod ssh_smoke;

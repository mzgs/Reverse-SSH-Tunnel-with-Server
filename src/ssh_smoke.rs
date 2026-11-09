//! Optional real OpenSSH test. Uses temporary keys and loopback-only listeners.
use super::{Profile, Session, Status};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Fixture {
    directory: PathBuf,
    sshd: Option<Child>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(child) = &mut self.sshd {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn wait_for_exit_or_ready(session: &mut Session) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while session.status == Status::Connecting && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        session.poll();
    }
}

fn check_traffic(local: &TcpListener, remote_port: u16) {
    let mut remote = TcpStream::connect(("127.0.0.1", remote_port)).unwrap();
    remote
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    remote.write_all(b"through the reverse tunnel").unwrap();
    local.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut service = loop {
        if let Ok((stream, _)) = local.accept() {
            break stream;
        }
        assert!(Instant::now() < deadline, "local service was not reached");
        std::thread::sleep(Duration::from_millis(20));
    };
    service
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut received = [0; 26];
    service.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"through the reverse tunnel");
    service.write_all(b"response").unwrap();
    let mut response = [0; 8];
    remote.read_exact(&mut response).unwrap();
    assert_eq!(&response, b"response");
}

#[test]
#[ignore = "requires /usr/sbin/sshd and permission to run a local SSH daemon"]
fn forwards_real_traffic_rejects_untrusted_hosts_and_reports_port_conflicts() {
    let directory = std::env::temp_dir().join(format!(
        "ssh-tunnel-plus-ssh-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    fs::create_dir(&directory).unwrap();
    let mut fixture = Fixture {
        directory: directory.clone(),
        sshd: None,
    };
    for key in ["host", "client"] {
        assert!(
            Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(directory.join(key))
                .status()
                .unwrap()
                .success()
        );
    }
    fs::copy(
        directory.join("client.pub"),
        directory.join("authorized_keys"),
    )
    .unwrap();
    let ssh_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let ssh_port = ssh_listener.local_addr().unwrap().port();
    drop(ssh_listener);
    let remote_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let remote_port = remote_listener.local_addr().unwrap().port();
    drop(remote_listener);
    let local = TcpListener::bind("127.0.0.1:0").unwrap();
    let local_port = local.local_addr().unwrap().port();
    let user = String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_string();
    let config = format!(
        "Port {ssh_port}\nListenAddress 127.0.0.1\nHostKey \"{0}/host\"\nAuthorizedKeysFile \"{0}/authorized_keys\"\nPidFile \"{0}/sshd.pid\"\nStrictModes no\nUsePAM no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPubkeyAuthentication yes\nPermitRootLogin yes\nAllowTcpForwarding remote\nGatewayPorts clientspecified\nAllowUsers {user}\n",
        directory.display()
    );
    fs::write(directory.join("sshd_config"), config).unwrap();
    fixture.sshd = Some(
        Command::new("/usr/sbin/sshd")
            .args(["-D", "-e", "-f"])
            .arg(directory.join("sshd_config"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(directory.join("sshd.log")).unwrap())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(("127.0.0.1", ssh_port)).is_err() {
        assert!(
            Instant::now() < deadline
                && fixture.sshd.as_mut().unwrap().try_wait().unwrap().is_none(),
            "sshd startup failed: {}",
            fs::read_to_string(directory.join("sshd.log")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let known_hosts = directory.join("known_hosts");
    let profile = Profile {
        id: 1,
        name: "Loopback integration".into(),
        server: "127.0.0.1".into(),
        user,
        ssh_port,
        identity: directory.join("client").to_string_lossy().into(),
        local_host: "127.0.0.1".into(),
        local_port,
        remote_port,
        public: false,
        local_command: String::new(),
        local_autostart: true,
        autostart: false,
    };
    let ssh_command = || {
        let mut command = Command::new("ssh");
        command.args([
            "-o".to_string(),
            format!("UserKnownHostsFile={}", known_hosts.display()),
            "-o".into(),
            "GlobalKnownHostsFile=/dev/null".into(),
            "-o".into(),
            "IdentityAgent=none".into(),
        ]);
        command.args(profile.ssh_args());
        command
    };
    let start = || Session::spawn(ssh_command()).unwrap();
    // The production strict-host-key policy must reject even a reachable server.
    let mut unknown = start();
    wait_for_exit_or_ready(&mut unknown);
    assert_eq!(unknown.status, Status::Failed, "{:?}", unknown.logs);
    assert!(
        unknown
            .logs
            .iter()
            .any(|line| line.contains("Host key verification failed"))
    );
    let public_key = fs::read_to_string(directory.join("host.pub")).unwrap();
    fs::write(&known_hosts, format!("[127.0.0.1]:{ssh_port} {public_key}")).unwrap();
    let mut tunnel = Session::start_ssh(ssh_command());
    wait_for_exit_or_ready(&mut tunnel);
    assert_eq!(tunnel.status, Status::Connected, "{:?}", tunnel.logs);
    check_traffic(&local, remote_port);
    let mut conflict = start();
    wait_for_exit_or_ready(&mut conflict);
    assert_eq!(conflict.status, Status::Failed, "{:?}", conflict.logs);
    assert!(
        conflict
            .logs
            .iter()
            .any(|line| line.contains("remote port forwarding failed"))
    );

    // Kill the client without requesting Disconnect, then recover real forwarding.
    let child = tunnel.child.as_mut().unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    tunnel.poll();
    assert_eq!(tunnel.status, Status::Reconnecting);
    let deadline = Instant::now() + Duration::from_secs(15);
    while tunnel.status != Status::Connected && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        tunnel.poll();
    }
    assert_eq!(tunnel.status, Status::Connected, "{:?}", tunnel.logs);
    check_traffic(&local, remote_port);

    tunnel.stop();
    assert!(!tunnel.active());
    assert!(tunnel.retry_at.is_none());
    let deadline = Instant::now() + Duration::from_secs(3);
    while TcpStream::connect(("127.0.0.1", remote_port)).is_ok() {
        assert!(
            Instant::now() < deadline,
            "remote listener survived disconnect"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

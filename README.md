# Portway

A native reverse SSH tunnel manager for **macOS, Windows, and Linux**, built with Rust and GPUI Kit. Create and edit saved profiles, run multiple tunnels, inspect connection logs, and disconnect with one click.

```text
SSH server :8080  ── encrypted SSH tunnel ──▶  this device :3000
```

## Run

Install [Rust stable](https://rustup.rs/) and OpenSSH **8.0 or later** (`ssh -V`).

```sh
cargo run --locked
```

- **macOS:** install the Xcode Command Line Tools (`xcode-select --install`). OpenSSH is included. `./run.sh` also starts the app; `./build.sh` produces an ad hoc signed `.app` in `target/release/`.
- **Windows 10/11:** use the Rust MSVC toolchain with Visual Studio C++ Build Tools and a Windows SDK. Enable the **OpenSSH Client** optional feature. `cargo build --release --locked` creates `target/release/reverse-ssh-tunnel-with-server.exe`.
- **Linux:** use a desktop session and a Vulkan-capable GPU/driver. On Ubuntu 24.04, install:

  ```sh
  sudo apt-get install gcc g++ clang libfontconfig-dev libwayland-dev \
    libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libasound2-dev \
    libvulkan1 mesa-vulkan-drivers openssh-client
  ```

  `./build.sh` or `cargo build --release --locked` creates `target/release/reverse-ssh-tunnel-with-server`.

GPUI Kit is pinned to version 0.9.0 (revision `0ab9cf4e`), using GPUI Fast 0.1.13. CI checks all three operating systems; local validation is on macOS.

## First connection

1. Start the service you want to forward, for example an app listening on `127.0.0.1:3000`.
2. On your SSH server, enable `AllowTcpForwarding yes` (or `remote`) in `sshd_config` and reload sshd. Your account and authorized key must also permit forwarding. You supply and administer this server; Portway does not install an SSH daemon.
3. Verify the server's host key once in a terminal. Replace the username, hostname, and port below. Verify the displayed fingerprint with your server administrator or provider before accepting it:

   ```sh
   ssh -F /dev/null -p 22 deploy@your-server
   ```

   On Windows, replace `/dev/null` with `NUL`. For a specific key, add `-i /absolute/path/to/key`. Exit the remote shell after verification. Portway uses your normal OpenSSH `known_hosts` file and rejects unknown or changed host keys.
4. Use an SSH agent (`ssh-add /path/to/key`) or select a private key using **Browse**. Encrypted keys must already be unlocked in the agent. No password/passphrase prompts are shown. A desktop app may not inherit the same agent environment as your terminal; launch from that terminal if necessary.
5. Click **New tunnel**, enter the server, username, local port `3000`, and remote port `8080`, then save and **Connect**. Once connected, `127.0.0.1:8080` **on your SSH server** forwards to your local app.

For access from other machines, explicitly enable **Allow public access**, configure `GatewayPorts clientspecified` on the server, and allow the remote port through its firewall. Protect the forwarded service with authentication. The server has the final say on bind addresses: `GatewayPorts yes` can force public binding even when a client requests loopback. For private access, use `GatewayPorts no` or `clientspecified`. See the [OpenSSH forwarding documentation](https://man.openbsd.org/ssh#R) and [server configuration reference](https://man.openbsd.org/sshd_config#GatewayPorts).

## Behavior and limits

- A tunnel becomes **Connected** only after OpenSSH confirms the remote forwarding request. This does not test the availability of your local service or a public firewall rule.
- Keepalives detect a lost server connection. Failures remain visible in the log; reconnect manually. Closing or quitting the app stops and reaps its SSH child processes. Forced process termination or an OS crash can bypass cleanup.
- Only key/default-key/agent authentication is supported. No password storage, SSH agent forwarding, automatic reconnect, jump hosts, SSH config aliases, background service, or server provisioning. Portway ignores user SSH config to avoid implicit forwards, proxy commands, and detached processes.
- Logs are held in memory, capped at 200 lines per tunnel, and support text selection and copying. They may contain usernames, hostnames, and key paths; review before sharing.
- Profiles store connection details and key **paths**, never key contents, in `settings.json` under the platform's local app-data directory named `reverse-ssh-tunnel-with-server`. Window geometry is stored alongside it. Invalid files are preserved and saving is disabled with an error message. Up to 100 profiles are supported.

## Check

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Tests cover profile validation and persistence, SSH arguments, forwarding readiness, bounded logs, child-process cleanup, window geometry, and the profile editor's save/cancel/validation flow.

The optional loopback integration test starts a temporary `/usr/sbin/sshd` with throwaway keys, without changing your SSH configuration or known hosts. On a Unix machine with sshd installed and permission to run it:

```sh
cargo test --locked forwards_real_traffic -- --ignored
```

It checks traffic in both directions, unknown-host rejection, remote-port conflicts, and listener cleanup after disconnect.

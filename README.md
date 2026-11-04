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

## Start a local service automatically

Set **Local service command (optional)** in a tunnel profile, for example:

```sh
cd ~/my-app && npm start
```

For every saved, nonblank command, Portway first checks the configured local host and port. If a TCP connection succeeds, it uses the existing service and skips the command. Otherwise it starts the command. This check runs when Portway opens, when you connect, and when you start the service. Running services show a green **Running** status; **Start service** / **Stop service** sits at the right of the service-status row, including for services started elsewhere. It checks only the port, without inspecting processes or sending application requests. Leave the command blank to manage the service yourself. **Stop service** also force-terminates processes listening on the configured local TCP port on this computer. This stops the whole process, including any other work it handles. Services on other computers cannot be stopped. On macOS/Linux this requires `lsof`; Windows uses PowerShell and `taskkill`. Permission or restart failures appear in the app. Closing or quitting stops only local services and SSH processes Portway starts.

Enable **Connect SSH when Portway opens** in a tunnel profile to connect it automatically at app startup. It is off by default, including for existing profiles. Any configured local service is checked or started before SSH is launched; its application readiness is not checked. SSH failures remain visible in the log or error message; retry with **Connect** once the network and SSH agent are ready.

On macOS, commands run in a noninteractive login `/bin/zsh`, starting in your home directory; include `cd` for the project folder. Linux uses `/bin/sh -lc`; Windows uses `cmd.exe /D /S /C`. Use full executable paths if your shell's login configuration does not put your tools on PATH. Commands run with your account's permissions, so only save commands you trust.

Keep the command in the foreground (no `&`, `nohup`, or daemon mode) so Portway can track it and stop its subprocesses. The tunnel details show service status, **Start service** / **Stop service**, and stdout/stderr in the connection log. **Edit** remains available while a profile is running. **Save and reconnect** saves the changes, stops that profile’s SSH tunnel and local service, then starts them with the saved settings, even if startup connection is disabled. Canceling, invalid input, and failed saves leave the running processes untouched. Other profiles keep running. Closing Portway force-stops the commands and their subprocesses; it does not wait for graceful shutdown. A running process does not confirm the service is ready or listening; failures remain in the log and require a manual restart.

Portway enables **launch at login** by default, including for existing profiles. Open **Settings…** in the sidebar or app menu (Command+, on macOS, Control+, elsewhere) and use **Start Portway when computer starts** to enable or disable it. The checkbox starts checked, and changes save immediately. On macOS, build the `.app` with `./build.sh`, move it to a permanent location (such as `/Applications`), then open it; registration requires an app bundle and macOS 13 or later. Windows registers startup for the current user, and Linux uses an XDG autostart entry. This opens the app after you sign in, then starts saved local commands and connects tunnels with **Connect SSH when Portway opens** enabled. It does not run before login.

You can also disable startup in your OS settings; Portway applies registration only on first launch or when you change its checkbox. The checkbox stores Portway’s preference; OS overrides remain in effect until you toggle it off and back on. Registration state is saved in `startup.json` alongside `settings.json`. Keep the application in its registered location; after moving the executable, remove `startup.json` and open Portway again to register the new location. Registration failures appear in the app and are retried the next time it opens. Linux requires a desktop environment that supports XDG autostart.

## Behavior and limits

- A tunnel becomes **Connected** only after OpenSSH confirms the remote forwarding request. This does not test the availability of your local service or a public firewall rule.
- Keepalives detect a lost server connection. Failures remain visible in the log; reconnect manually. Closing or quitting the app stops and reaps its SSH child processes. Forced process termination or an OS crash can bypass cleanup.
- Only key/default-key/agent authentication is supported. No password storage, SSH agent forwarding, automatic reconnect, jump hosts, SSH config aliases, background service, or server provisioning. Portway ignores user SSH config to avoid implicit forwards, proxy commands, and detached processes.
- Logs are held in memory, capped at 200 lines per process, and support text selection and copying. They may contain usernames, hostnames, key paths, and local command output; review before sharing.
- Profiles store connection details and key **paths**, never key contents, in `settings.json` under the platform's local app-data directory named `reverse-ssh-tunnel-with-server`. Window geometry is stored alongside it. Invalid files are preserved and saving is disabled with an error message. Up to 100 profiles are supported.

## Check

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Tests cover profile validation and persistence, SSH arguments, forwarding readiness, bounded logs, local command startup and output, child-process cleanup, window geometry, and the profile editor's save/cancel/validation flow.

The optional loopback integration test starts a temporary `/usr/sbin/sshd` with throwaway keys, without changing your SSH configuration or known hosts. On a Unix machine with sshd installed and permission to run it:

```sh
cargo test --locked forwards_real_traffic -- --ignored
```

It checks traffic in both directions, unknown-host rejection, remote-port conflicts, and listener cleanup after disconnect.

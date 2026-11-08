# SSH Tunnel Plus

A desktop app for **macOS, Windows, and Linux** that makes a local service reachable through an SSH server you control. Save connection profiles, connect multiple reverse SSH tunnels, and view their logs in one window.

Use it to reach a development app or another local service through your server, without opening an inbound port on your own computer or router.

```text
On your SSH server                         On your computer
127.0.0.1:8080  ── encrypted SSH tunnel ──▶  127.0.0.1:3000
```

**[Download the latest release](https://github.com/mzgs/Reverse-SSH-Tunnel-with-Server/releases/latest)** · [All releases](https://github.com/mzgs/Reverse-SSH-Tunnel-with-Server/releases) · [First connection](#first-connection) · [Build from source](#build-from-source)

## What you can do

- Save and edit tunnel profiles, and run several tunnels at once.
- Connect and disconnect with one click, with connection status and copyable logs.
- Start a local service with a saved command before connecting.
- Choose which tunnels connect when the app opens, and whether the app launches at login.
- Keep the server port private by default, or explicitly enable public access.

You need your own reachable SSH server and SSH key access. SSH Tunnel Plus uses OpenSSH on your computer; it does not provide a hosted server or support password prompts.

## Download and install

**Release builds do not require Rust or compiling.** Open the [latest release](https://github.com/mzgs/Reverse-SSH-Tunnel-with-Server/releases/latest), expand **Assets**, and download the archive for your computer. Choose a file from this table, rather than GitHub's **Source code** archives. If no release is available, you can [build from source](#build-from-source).

`<VERSION>` below is the version shown on the release page.

| Your computer | Download |
| --- | --- |
| macOS with Apple Silicon (M-series) | `ssh-tunnel-plus-v<VERSION>-macos-arm64.zip` |
| macOS with an Intel processor | `ssh-tunnel-plus-v<VERSION>-macos-x86_64.zip` |
| Windows 10/11, x86-64 (Intel/AMD) | `ssh-tunnel-plus-v<VERSION>-windows-x86_64.zip` |
| Linux x86-64 (Ubuntu 24.04 or a compatible newer desktop) | `ssh-tunnel-plus-v<VERSION>-linux-x86_64.tar.gz` |

All platforms require **OpenSSH 8.0 or later** on PATH. Check it in Terminal or PowerShell with `ssh -V`.

### macOS

1. Download the ZIP for your processor. Check **Apple menu → About This Mac** if unsure.
2. Extract it and move **SSH Tunnel Plus.app** to **Applications** before opening it.
3. Open the app. The bundle is ad hoc signed and is not notarized by Apple, so macOS may block the first launch. If you trust the download, use **System Settings → Privacy & Security → Open Anyway** after trying to open it, then confirm.

OpenSSH is included with macOS. Launch at login requires macOS 13 or later.

### Windows

1. Download the Windows ZIP and choose **Extract All**. Keep the extracted folder in a permanent location.
2. Ensure the **OpenSSH Client** optional feature is installed, and check `ssh -V` in PowerShell.
3. Open **reverse-ssh-tunnel-with-server.exe** from the extracted folder.

The executable is unsigned. If Windows SmartScreen blocks it, review the download's source and use **More info → Run anyway** only if you trust it and that option is available.

### Linux

1. Download and extract the Linux `.tar.gz` archive into a permanent folder.
2. Use an Ubuntu 24.04 or compatible newer desktop with a Vulkan-capable GPU/driver and OpenSSH installed. On Ubuntu, `sudo apt-get install openssh-client libvulkan1 mesa-vulkan-drivers lsof` installs SSH, Vulkan runtime support, and the tool used to stop local services. Other shared libraries may also be needed; see the [Linux dependencies](#build-from-source).
3. Open a terminal in the extracted folder and run:

   ```sh
   chmod +x reverse-ssh-tunnel-with-server
   ./reverse-ssh-tunnel-with-server
   ```

### After opening

The app enables **launch at login by default**. To change it, open **Settings…** and toggle **Start SSH Tunnel Plus when computer starts**. Individual tunnels connect at startup only when you enable that option in their profiles. Keep the app in its installed location so the startup entry continues to work.

Releases include `SHA256SUMS` for checking downloaded archives. To update, quit the app, extract the newer release, and replace the old app or executable in the same location. Saved profiles live in your local app-data directory, separately from the download.

## First connection

1. Start the service you want to forward, for example an app listening on `127.0.0.1:3000`.
2. On your SSH server, enable `AllowTcpForwarding yes` (or `remote`) in `sshd_config` and reload sshd. Your account and authorized key must also permit forwarding. You supply and administer this server; SSH Tunnel Plus does not install an SSH daemon.
3. Verify the server's host key once in a terminal. Replace the username, hostname, and port below. Verify the displayed fingerprint with your server administrator or provider before accepting it:

   ```sh
   ssh -F /dev/null -p 22 deploy@your-server
   ```

   On Windows, replace `/dev/null` with `NUL`. For a specific key, add `-i /absolute/path/to/key`. Exit the remote shell after verification. SSH Tunnel Plus uses your normal OpenSSH `known_hosts` file and rejects unknown or changed host keys.
4. Use an SSH agent (`ssh-add /path/to/key`) or select a private key using **Browse**. Encrypted keys must already be unlocked in the agent. No password/passphrase prompts are shown. A desktop app may not inherit the same agent environment as your terminal; launch from that terminal if necessary.
5. Click **New tunnel**, enter the server, username, local port `3000`, and remote port `8080`, then save and **Connect**. Once connected, `127.0.0.1:8080` **on your SSH server** forwards to your local app.

For access from other machines, explicitly enable **Allow public access**, configure `GatewayPorts clientspecified` on the server, and allow the remote port through its firewall. Protect the forwarded service with authentication. The server has the final say on bind addresses: `GatewayPorts yes` can force public binding even when a client requests loopback. For private access, use `GatewayPorts no` or `clientspecified`. See the [OpenSSH forwarding documentation](https://man.openbsd.org/ssh#R) and [server configuration reference](https://man.openbsd.org/sshd_config#GatewayPorts).

## Start a local service automatically

Set **Local service command (optional)** in a tunnel profile, for example:

```sh
cd ~/my-app && npm start
```

For every saved, nonblank command, SSH Tunnel Plus first checks the configured local host and port. If a TCP connection succeeds, it uses the existing service and skips the command. Otherwise it starts the command. This check runs when SSH Tunnel Plus opens, when you connect, and when you start the service. Running services show a green **Running** status; **Start service** / **Stop service** sits at the right of the service-status row, including for services started elsewhere. External service status is refreshed in the background without starting the command again. Status checks only connect to the port, without inspecting processes or sending application requests. Leave the command blank to manage the service yourself. **Stop service** also force-terminates processes listening on the configured local address and TCP port on this computer, including wildcard listeners in the same address family. For a service listening on IPv6 (including dual-stack `::`), use an IPv6 local host such as `::1` to stop it; the app will not guess across address families. This stops the whole process, including any other work it handles. Services on other computers cannot be stopped. On macOS/Linux this requires `lsof`; Windows uses PowerShell and `taskkill`. Permission or restart failures appear in the app. Closing or quitting stops only local services and SSH processes SSH Tunnel Plus starts.

Enable **Connect SSH when SSH Tunnel Plus opens** in a tunnel profile to connect it automatically at app startup. It is off by default, including for existing profiles. Any configured local service is checked or started before SSH is launched; its application readiness is not checked. Once started, SSH retries automatically after failures, including while the network or SSH agent is unavailable. Invalid profiles and missing identity files must be corrected before connecting.

On macOS, commands run in a noninteractive login `/bin/zsh`, starting in your home directory; include `cd` for the project folder. Linux uses `/bin/sh -lc`; Windows uses `cmd.exe /D /S /C`. Use full executable paths if your shell's login configuration does not put your tools on PATH. Commands run with your account's permissions, so only save commands you trust.

Keep the command in the foreground (no `&`, `nohup`, or daemon mode) so SSH Tunnel Plus can track it and stop its subprocesses. The tunnel details show service status, **Start service** / **Stop service**, and stdout/stderr in the connection log. **Edit** remains available while a profile is running. **Save and reconnect** saves the changes, stops that profile’s SSH tunnel and local service, then starts them with the saved settings, even if startup connection is disabled. Canceling, invalid input, and failed saves leave the running processes untouched. Other profiles keep running. Closing SSH Tunnel Plus force-stops the commands and their subprocesses; it does not wait for graceful shutdown. A running process does not confirm the service is ready or listening; failures remain in the log and require a manual restart.

SSH Tunnel Plus enables **launch at login** by default, including for existing profiles. Open **Settings…** in the sidebar or app menu (Command+, on macOS, Control+, elsewhere) and use **Start SSH Tunnel Plus when computer starts** to enable or disable it. The checkbox starts checked, and changes save immediately. On macOS, install the release `.app` in a permanent location (such as `/Applications`) and open it; registration requires an app bundle and macOS 13 or later. Windows registers startup for the current user, and Linux uses an XDG autostart entry. This opens the app after you sign in, then starts saved local commands and connects tunnels with **Connect SSH when SSH Tunnel Plus opens** enabled. It does not run before login.

You can also disable startup in your OS settings; SSH Tunnel Plus applies registration only on first launch or when you change its checkbox. The checkbox stores SSH Tunnel Plus’s preference; OS overrides remain in effect until you toggle it off and back on. Registration state is saved in `startup.json` alongside `settings.json`. Keep the application in its registered location; after moving the executable, remove `startup.json` and open SSH Tunnel Plus again to register the new location. Windows startup entries keep the legacy name `Portway`; Linux keeps `Portway.desktop` as its internal filename. Registration failures appear in the app and are retried the next time it opens. Linux requires a desktop environment that supports XDG autostart.

## Behavior and limits

- A tunnel becomes **Connected** only after OpenSSH confirms the remote forwarding request. This does not test the availability of your local service or a public firewall rule.
- SSH keepalives run every 15 seconds and detect an unresponsive connection after three unanswered probes. Failed connections show **Reconnecting** and retry every five seconds without a retry limit while the app is open. Connected tunnels have no lifetime limit. **Disconnect** cancels retries. Sleep, network outages, and server failures can interrupt traffic; reconnecting restores the tunnel but existing TCP connections must be reopened by their applications. Closing or quitting the app stops retries and reaps its SSH child processes. Forced process termination or an OS crash can bypass cleanup.
- Only key/default-key/agent authentication is supported. No password storage, SSH agent forwarding, jump hosts, SSH config aliases, background service, or server provisioning. SSH Tunnel Plus ignores user SSH config to avoid implicit forwards, proxy commands, and detached processes. Host-key verification stays strict on every retry; authentication and host-key failures remain in the log and require correcting the key, agent, or trusted host entry.
- Logs are held in memory, capped at 200 lines per process, and support text selection and copying. They may contain usernames, hostnames, key paths, and local command output; review before sharing.
- Profiles store connection details and key **paths**, never key contents, in `settings.json` under the platform's local app-data directory named `reverse-ssh-tunnel-with-server`. Window geometry is stored alongside it. Invalid files are preserved and saving is disabled with an error message. Up to 100 profiles are supported.

## Build from source

Install [Rust stable](https://rustup.rs/), Git, and OpenSSH **8.0 or later** (`ssh -V`), plus the platform dependencies below. Then:

```sh
git clone https://github.com/mzgs/Reverse-SSH-Tunnel-with-Server.git
cd Reverse-SSH-Tunnel-with-Server
cargo run --locked
```

- **macOS:** install the Xcode Command Line Tools (`xcode-select --install`). OpenSSH is included. `./run.sh` also starts the app; `./build.sh` produces the ad hoc signed `target/release/SSH Tunnel Plus.app`.
- **Windows 10/11:** use the Rust MSVC toolchain with Visual Studio C++ Build Tools and a Windows SDK. Enable the **OpenSSH Client** optional feature. `cargo build --release --locked` creates `target/release/reverse-ssh-tunnel-with-server.exe`.
- **Linux:** use a desktop session and a Vulkan-capable GPU/driver. On Ubuntu 24.04, install:

  ```sh
  sudo apt-get install gcc g++ clang libfontconfig-dev libwayland-dev \
    libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libasound2-dev \
    libvulkan1 mesa-vulkan-drivers openssh-client
  ```

  `./build.sh` or `cargo build --release --locked` creates `target/release/reverse-ssh-tunnel-with-server`.

GPUI Kit is pinned to version 0.9.0 (revision `0ab9cf4e`), using GPUI Fast 0.1.13. The release workflow builds the four platform variants listed above; local validation is on macOS.

## Development checks

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Tests cover profile validation and persistence, SSH arguments, forwarding readiness, repeated reconnects after exits and timeouts, retry cancellation, bounded logs, local command startup and output, child-process cleanup, window geometry, and the profile editor's save/cancel/validation flow.

The optional loopback integration test starts a temporary `/usr/sbin/sshd` with throwaway keys, without changing your SSH configuration or known hosts. On a Unix machine with sshd installed and permission to run it:

```sh
cargo test --locked forwards_real_traffic -- --ignored
```

It checks traffic in both directions, unknown-host rejection, remote-port conflicts, automatic recovery after killing the SSH client, and listener cleanup after disconnect.

## Publishing a release (maintainers)

Commit the release tooling and your changes first. With Python **3.11+**, Git, and a clean working tree on a branch that already exists on `origin`, run:

```sh
python3 release.py --dry-run  # Preview the next patch version; checks origin too
python3 release.py            # For example, 0.1.0 → 0.1.1
```

The script updates only this app's version in `Cargo.toml` and `Cargo.lock`, commits it, creates an annotated `v…` tag, and atomically pushes the current branch and tag. It uses Git's existing `origin` credentials (including a token already stored in `.git/config`); no token is copied into source files. The token must permit repository pushes and workflow changes when first adding the workflow. The macOS bundle version follows Cargo automatically.

The [release workflow](.github/workflows/release.yml) builds natively using GitHub's [hosted runners](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job). Once every build succeeds, it [publishes a GitHub Release](https://cli.github.com/manual/gh_release_create) with generated notes, `SHA256SUMS`, and the four platform archives listed in [Download and install](#download-and-install).

GitHub Actions uses its built-in `GITHUB_TOKEN` to publish; no additional release secret is required. Watch the repository's **Actions → Release** page. If a build fails, fix its cause and rerun the failed jobs, or commit the fix and create another patch release. If the push fails, the script keeps the local release commit and tag and prints the exact push command to retry; it does not reset your work.

macOS bundles are ad hoc signed, without Apple notarization; Windows binaries are unsigned. Linux binaries target Ubuntu 24.04 or a compatible newer desktop and need the runtime libraries listed above. OpenSSH remains a runtime requirement on every platform. Validate the release tooling locally with `python3 -m unittest test_release`; this uses a temporary local Git remote and never publishes anything.

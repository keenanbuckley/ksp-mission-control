# ksp-mission-control

A Rust + Axum dashboard that plans and executes Kerbal Space Program launches via kRPC and kOS.

## What it does

The server fronts a running KSP install and serves a small dashboard at `http://127.0.0.1:8080`. The dashboard exposes four buttons:

- **Launch** runs `launch.ks` in-game, taking the active vessel up to a low orbit target apoapsis.
- **Plan Circ** computes a circularization maneuver node server-side and queues it on the vessel.
- **Execute Node** runs `maneuver.ks` to perform the next queued burn.
- **Toggle AG1** flips Action Group 1.

The Launch -> Plan Circ -> Execute Node sequence is the current demo: ascend, plan, circularize, all without leaving the browser.

## Requirements

- Kerbal Space Program (any recent 1.12.x build).
- KSP mods:
  - [kRPC](https://krpc.github.io/krpc/) - RPC bridge between KSP and the server. The server connects to its default ports: `127.0.0.1:50000` (RPC) and `127.0.0.1:50001` (stream).
  - [kOS](https://ksp-kos.github.io/KOS/) - in-game scripting runtime.
  - kIPC - kRPC <-> kOS bridge. The Rust server uses it via the `kipc::KIPC` API, and the kerboscript dispatcher uses it via `ADDONS:KIPC`. Install from either the [keenanbuckley/ksp-kipc](https://github.com/keenanbuckley/ksp-kipc) or [roxik0/ksp-kipc](https://github.com/roxik0/ksp-kipc) fork; the upstream `dewiniaid/ksp-kipc` is abandoned.
- Rust toolchain (stable, edition 2021).

## Install

```sh
git clone https://github.com/keenanbuckley/ksp-mission-control.git
cd ksp-mission-control
cargo build --release
```

Deploy the kerboscript to your KSP install:

```sh
cargo run --bin deploy-kos
```

On first run, `deploy-kos` prompts for the KSP `Ships/Script/` directory and writes the answer to `.kos.toml`; subsequent runs reuse it. The destination can also be passed as `--path <dir>` or set via `KSP_SCRIPT_DIR=<dir>`. This copies the `.ks` files from `kos/scripts/` into the KSP scripts directory (which kOS exposes as the archive volume, `0:`), preserving the `boot/` and `lib/` subdirectories.

In-game, on the kOS processor of the vessel you want to drive, set its boot file to the deployed `boot/dispatch_listener.ks` (path `0:/boot/dispatch_listener.ks` on the archive volume).

## Run

Start the server:

```sh
cargo run
```

The server binds `http://127.0.0.1:8080` and supervises the kRPC connection in the background. Order between KSP and the server does not matter: the supervisor retries kRPC with exponential backoff (logs `kRPC connect failed; retrying`), and the dashboard shows "KSP: not connected" until kRPC at `127.0.0.1:50000` / `50001` is reachable. Once it is, the dashboard auto-promotes to "KSP: connected" and Universal Time starts ticking.

In KSP, load a vessel with a kOS processor running the boot script (see Install). The buttons activate as soon as kRPC reports the vessel.

The Launch -> wait for apoapsis -> Plan Circ -> Execute Node sequence drives the vessel from the pad into a circular orbit.

## Rocket-design notes

The launch script makes a few assumptions about the vessel:

- **AG1 fires automatically at the edge of space.** Once surface velocity passes ~1 km/s with dynamic pressure under `0.01`, the script triggers Action Group 1. Bind deploy-on-exit-atmosphere parts to AG1 when assembling the rocket: fairing decouplers, comms antennas, solar panels. The Toggle AG1 button is wired to the same group for manual firing.
- **Launch clamps drive the countdown.** The script counts how many staging events it takes to release all `LaunchClamp` parts from the current stage and spreads them across the countdown so the lowest-numbered clamp stage fires at T-0. Any pre-clamp stages (typically main-engine ignition) fire on the ticks before. Use launch clamps for KSC-style launches.
- **Auto-stages on flameout or zero thrust.** A continuous trigger stages whenever `maxThrust` reaches zero or any engine flames out. Order the staging so each spent engine cluster decouples cleanly into its own stage.
- **Throttle is locked to a target TWR (default 2.0).** Insufficient thrust pegs the throttle at 100% and slows the ascent; excess thrust is throttled down. Size first-stage engines to clear the target with headroom.

## Architecture

The server is a single Rust process. Modules wired by tokio channels: `src/krpc.rs` owns the kRPC client and streams and publishes telemetry over a `broadcast::Sender`; `src/web.rs` is the Axum WebSocket handler that fans telemetry out to connected browsers; `src/main.rs` is the wiring. `src/planning.rs` holds the server-side circularization math, and `src/control.rs` holds the kIPC command dispatcher.

The browser is plain HTML + vanilla JS (`static/index.html`), no build step. kOS scripts live under `kos/scripts/` with `boot/` and `lib/` subdirectories. The server pushes named ops (`run_script`, `plan_circ`, `toggle_ag`) over kIPC; the kerboscript dispatcher replies with `command_ack`, `script_done`, and `command_error` events.

## License

MIT. See [LICENSE](LICENSE).

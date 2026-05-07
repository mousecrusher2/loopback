# Pico 2 UAC1 Loopback

USB Audio Class 1.0 stereo loopback firmware for Raspberry Pi Pico 2.

See [docs/uac1-design.md](docs/uac1-design.md) for the descriptor and
synchronization decisions.

Supported alternate settings:

- 16-bit PCM: 44.1 kHz, 48 kHz, 88.2 kHz, 96 kHz
- 24-bit PCM: 44.1 kHz, 48 kHz, 88.2 kHz, 96 kHz
- 32-bit PCM: 44.1 kHz, 48 kHz

Build:

```powershell
cargo build --release
```

Run the embedded unit tests over the debug probe:

```powershell
cargo test
```

These tests flash a test firmware through `probe-rs`. They do not require the
target USB port to be connected.

Flash/run with `probe-rs`:

```powershell
cargo run --release
```

The runner is configured for `probe-rs run --chip RP235x`.

Host-side bit-perfect loopback checks live in
[tools/loopback-check](tools/loopback-check). With the target USB connected:

```powershell
cargo loopback-check -- list
cargo loopback-check -- test --rate 48000 --bits 16 --seconds 3
cargo loopback-check -- test --rate 48000 --bits 24 --timing events --capture-mode shared
cargo loopback-check -- test --rate 48000 --bits 32 --timing events
```

Shared capture tests set the selected Windows capture shared-mode format to the
requested rate/bit depth by default, then restore the previous setting.

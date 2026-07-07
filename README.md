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

Build UF2:

```powershell
pwsh -File scripts/build-uf2.ps1
```

The UF2 is written to `target/uf2/pico2-uac1-loopback.uf2`.

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

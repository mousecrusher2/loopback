# loopback-check

Windows WASAPI checker for the Pico 2 UAC1 loopback firmware.

List endpoints:

```powershell
cargo loopback-check -- list
```

Run one bit-perfect check:

```powershell
cargo loopback-check -- test --rate 48000 --bits 16 --seconds 3 --dump-dir artifacts/loopback-48k16
```

Run with WASAPI shared-mode capture while keeping render exclusive:

```powershell
cargo loopback-check -- test --rate 48000 --bits 24 --timing events --capture-mode shared
```

Run the full firmware matrix:

```powershell
cargo loopback-check -- matrix --seconds 3 --dump-dir artifacts/loopback-matrix
```

The checker opens the render endpoint in WASAPI exclusive mode. Capture defaults
to exclusive mode, or can be switched to shared mode with `--capture-mode shared`.
Shared capture uses `autoconvert=false`; if Windows would need a different shared
format, the test fails instead of comparing converted data. The checker writes
deterministic integer PCM, records the returned stream, aligns by exact byte sync,
and fails unless the aligned payload is byte-for-byte identical.

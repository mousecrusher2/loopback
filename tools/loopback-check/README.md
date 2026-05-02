# loopback-check

Windows WASAPI exclusive-mode checker for the Pico 2 UAC1 loopback firmware.

List endpoints:

```powershell
cargo loopback-check -- list
```

Run one bit-perfect check:

```powershell
cargo loopback-check -- test --rate 48000 --bits 16 --seconds 3 --dump-dir artifacts/loopback-48k16
```

Run the full firmware matrix:

```powershell
cargo loopback-check -- matrix --seconds 3 --dump-dir artifacts/loopback-matrix
```

The checker opens the render and capture endpoints in WASAPI exclusive polling
mode. It rejects format conversion, writes deterministic integer PCM, records
the returned stream, aligns by exact byte sync, and fails unless the aligned
payload is byte-for-byte identical.

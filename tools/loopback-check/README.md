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

For shared capture, the checker defaults to changing the selected capture
endpoint's Windows shared-mode device format to the requested rate/bit depth
before the test and restoring the previous setting afterwards. Use
`--shared-format leave` to only use the current Windows setting, or
`--shared-format set-keep` to leave the requested format applied after the test.
The tool first tries the documented endpoint property store. If Windows denies
read/write access, it falls back to the undocumented `IPolicyConfig` interface
used by the Windows audio settings path.

Run the full firmware matrix:

```powershell
cargo loopback-check -- matrix --seconds 3 --dump-dir artifacts/loopback-matrix
```

The checker opens the render endpoint in WASAPI exclusive mode. Capture defaults
to exclusive mode, or can be switched to shared mode with `--capture-mode shared`.
Shared capture uses `autoconvert=false`; if Windows would need conversion beyond
the selected shared-mode device format, the test fails instead of comparing
converted data. The checker writes deterministic integer PCM, records the
returned stream, aligns by exact byte sync, and fails unless the aligned payload
is byte-for-byte identical.

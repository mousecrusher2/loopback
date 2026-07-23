# Pico 2 UAC1 Loopback

Raspberry Pi Pico 2 用の USB Audio Class 1.0 ステレオループバック
ファームウェアです。

対応する代替設定：

- 16 ビット PCM：44.1 kHz、48 kHz、88.2 kHz、96 kHz
- 24 ビット PCM：44.1 kHz、48 kHz、88.2 kHz、96 kHz
- 32 ビット PCM：44.1 kHz、48 kHz

LED 診断：

Pico 2 のオンボード LED（GPIO 25）は、PC から受け取った再生音声を録音側へ
返せなかったときに約 1 秒間点灯します。同じ状態が続く間は点灯し続けます。
再生していない状態で録音だけを開始した場合も点灯します。

ビルド：

```powershell
cargo build --release
```

UF2 の生成：

```powershell
pwsh -File scripts/build-uf2.ps1
```

生成された UF2 は `target/uf2/pico2-uac1-loopback.uf2` に出力されます。

デバッグプローブ経由で組み込み単体テストを実行：

```powershell
cargo test
```

このテストでは、`probe-rs` を介してテスト用ファームウェアを
書き込みます。ターゲットの USB ポートを接続する必要はありません。

`probe-rs` で書き込み・実行：

```powershell
cargo run --release
```

Cargo ランナーは `probe-rs run --chip RP235x` に設定されています。

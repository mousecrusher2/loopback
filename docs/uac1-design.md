# UAC1 design notes

This firmware exposes one USB Audio Class 1.0 function with three interfaces:

- AudioControl interface
- AudioStreaming OUT interface for host playback into the device
- AudioStreaming IN interface for host capture from the device

The AudioControl topology intentionally presents a generic speaker path and a
generic microphone path, because current desktop hosts classify these terminal
types reliably. The firmware data path loops the OUT byte stream back into the
IN endpoint when both active alternate settings use the same sample width and
the same sampling frequency. If the host opens mismatched formats, the IN stream
returns digital silence instead of reinterpreting the byte stream.

## Formats

Each AudioStreaming interface has alternate setting 0 as the zero-bandwidth
setting. Alternate setting 1 is stereo 16-bit PCM, and alternate setting 2 is
stereo 24-bit PCM. Alternate setting 3 is stereo 32-bit PCM.

The 16-bit and 24-bit alternate settings advertise four discrete sample rates:
44.1 kHz, 48 kHz, 88.2 kHz, and 96 kHz. The 32-bit alternate setting advertises
44.1 kHz and 48 kHz only. The Type I Format Type descriptor uses `bSamFreqType`
followed by 24-bit little-endian `tSamFreq` entries, as defined by USB Audio
Data Formats 1.0 section 2.2.5.

The allocated RP2350 endpoint buffer is sized for the largest advertised packet,
24-bit stereo at 96 kHz plus one audio frame of headroom:

```text
97 samples/ms * 2 channels * 3 bytes = 582 bytes
```

The descriptor `wMaxPacketSize` is still written per alternate setting:

- 16-bit stereo at 96 kHz plus one audio frame of headroom: 388 bytes
- 24-bit stereo at 96 kHz plus one audio frame of headroom: 582 bytes
- 32-bit stereo at 48 kHz plus one adaptive OUT frame of headroom: 392 bytes

## Synchronization and feedback

The AudioStreaming OUT data endpoints are adaptive isochronous endpoints. The
host chooses the packet cadence, and the firmware accepts the packet sizes that
arrive for the active format.

The AudioStreaming IN data endpoints are asynchronous isochronous endpoints. The
firmware chooses the packet cadence with a millisecond packet accumulator for
44.1/88.2 kHz and exact frame counts for 48/96 kHz.

This firmware does not implement an explicit feedback endpoint.

That is deliberate. USB Audio Class 1.0 section 3.7.2.2 requires explicit synch
endpoints for adaptive audio source endpoints and asynchronous audio sink
endpoints. An asynchronous OUT sink would need a feedback IN endpoint carrying
3-byte 10.14-format `Ff` values at the advertised `bRefresh` cadence. That model
is correct when the device has an independent audio master clock and the host
must chase it.

The Pico 2 loopback firmware has no independent DAC/ADC/I2S master clock. It can
accept host-paced OUT packets and generate IN packets from USB frame cadence.
Advertising asynchronous OUT plus feedback would therefore imply a clock
relationship that this firmware does not actually own.

If this project later grows an external I2S codec clock or another free-running
audio clock, the sync model should be revisited. At that point, an asynchronous
OUT sink with explicit feedback, or a different clock-domain crossing strategy,
would be appropriate.

## Endpoint controls

The class-specific endpoint descriptor advertises Sampling Frequency Control.
`SET_CUR` stores the closest supported discrete sample rate, matching UAC1
section 5.2.3.2.3.1 guidance for unsupported discrete values. `GET_CUR`,
`GET_MIN`, `GET_MAX`, and `GET_RES` return three-byte little-endian frequency
values.

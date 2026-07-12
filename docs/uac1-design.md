# UAC1 design notes

This document records both the chosen design and its intentional limitations.
Some of the limitations could be made stricter, but doing so would add state and
cancellation protocols without improving the normal host/device combination for
which this firmware is intended.

## USB topology and loopback contract

The firmware exposes one USB Audio Class 1.0 function with three interfaces:

- AudioControl
- AudioStreaming OUT for host playback into the device
- AudioStreaming IN for host capture from the device

The AudioControl topology presents a generic speaker path and a generic
microphone path because desktop hosts classify these terminal types reliably.
There is no USB serial string.

When both streaming interfaces select the same alternate setting and sample
rate, each complete OUT isochronous payload is queued and returned unchanged by
the corresponding IN endpoint. The firmware does not resample, split, merge, or
pad normal loopback packets. When the selections do not match, or no queued
packet is available, IN sends digital silence. It never reinterprets bytes from
one PCM width as another.

The host is trusted to packetize OUT according to the selected rate. The one
structural check made by the device is that a non-empty OUT payload contains a
whole number of audio frames. A malformed partial frame is dropped.

## Formats are defined once

Each AudioStreaming interface has alternate setting 0 as the zero-bandwidth
setting. The remaining alternate settings are derived from the order of
`PCM_FORMATS` in `src/spec.rs`:

- alternate setting 1: stereo 16-bit PCM at 44.1, 48, 88.2, or 96 kHz
- alternate setting 2: stereo 24-bit PCM at 44.1, 48, 88.2, or 96 kHz
- alternate setting 3: stereo 32-bit PCM at 44.1 or 48 kHz

`PCM_FORMATS` is the single source of truth for the format descriptors,
alternate-setting mapping, endpoint arrays, endpoint task count, packet sizes,
and descriptor buffer capacities. These values are derived rather than copied
into parallel tables and checked with assertions or constant-only tests.

Descriptor construction keeps interface numbers, endpoint addresses, audio
entity IDs, terminal types, and channel configurations in distinct semantic
types. They are converted to descriptor bytes only at the serialization
boundary, so terminal/source IDs cannot be accidentally interchanged with an
unrelated raw `u8` argument.

The Type I Format Type descriptor uses `bSamFreqType` followed by 24-bit
little-endian `tSamFreq` entries, as defined by USB Audio Data Formats 1.0
section 2.2.5. `wMaxPacketSize` is derived per format from its maximum rate plus
one audio frame of headroom:

- 16-bit stereo through 96 kHz: 388 bytes
- 24-bit stereo through 96 kHz: 582 bytes
- 32-bit stereo through 48 kHz: 392 bytes

The shared software packet type is therefore sized to 582 bytes, the maximum of
the derived per-format values.

## Synchronization and feedback

All USB transactions are scheduled by the host controller on USB frame
boundaries. The endpoint synchronization type describes the audio clock/data
rate relationship; it does not allow the device to choose when an IN token is
issued.

The OUT endpoints are advertised as adaptive sinks. The host chooses the OUT
packet sizes and the firmware accepts them, subject only to endpoint MPS and
audio-frame alignment. This is also intended to preserve the host's playback
packetization in paths such as Windows shared mode instead of claiming that the
device owns a separate audio clock. It is a compatibility and quality choice,
not a guarantee that a host mixer is bit-perfect.

The IN endpoints are advertised as asynchronous sources. During normal
loopback, an IN payload has exactly the length of the queued OUT payload. It is
therefore not promised to contain a fixed synchronous number of samples per
SOF, even though integral rates and a well-behaved host will normally produce a
fixed OUT packet size. The device cannot synchronize directly to a host
application clock; it can only participate in the USB synchronization model and
respond to host-scheduled transfers.

There is no explicit feedback endpoint. The board has no independent
DAC/ADC/I2S master clock for the host to chase, so asynchronous OUT plus
feedback would claim a clock relationship the firmware does not own. If a
free-running external audio clock is added later, this choice must be revisited.

## Silence packet timing

`PacketClock` is used only to size generated silence packets. It accumulates the
selected nominal rate per millisecond, producing the fractional cadence needed
for rates such as 44.1 and 88.2 kHz. Normal loopback packets bypass it and retain
the exact OUT payload length.

The clock deliberately counts silence packets only; it does not reconcile
silence with the lengths of preceding OUT packets. A long pattern alternating
between queue underruns and silence could therefore accumulate timing error.
That case is accepted because silence is an exceptional fallback: a stable
stream is expected to remain queued rather than alternate continuously between
audio and silence.

## Endpoint tasks and zero-copy queues

Every nonzero format alternate has its own OUT endpoint task, IN endpoint task,
and `zerocopy_channel`. A task's PCM width, frame size, and MPS are fixed when it
is spawned. This prevents an inactive endpoint task from accidentally using the
new global selection's larger MPS. Such a mismatch can make an endpoint return
`BufferOverflow` immediately and create an executor-starving retry loop.

A single dynamic dispatcher is intentionally not used. Persistent per-endpoint
tasks fit Embassy's execution model, separate format queues make the ownership
simple, and RP2350 has ample RAM for them.

The channels use `NoopRawMutex`. Both halves are owned by tasks spawned on the
same thread-mode executor, and USB interrupts only wake endpoint futures; no
interrupt handler or second core accesses channel state. `NoopRawMutex` being
`!Sync` expresses this restriction. If either half is later moved to another
executor, an interrupt, or RP2350 core 1, the mutex type must be changed to one
that provides the corresponding cross-context synchronization.

The channel stores preallocated `heapless::Vec` packet slots. OUT reads directly
into a sender grant and IN writes directly from a receiver grant, avoiding a
move of an owned packet through a conventional channel. `heapless::Vec` remains
the packet representation because it already provides fixed-capacity storage
with a runtime payload length.

Isochronous OUT cannot be backpressured and retried. If a format's queue is
full, the task still services the endpoint and discards the newly arrived
packet. `BufferOverflow` is treated as a broken internal MPS invariant and the
affected task exits instead of retrying an error that may complete immediately.

## Alternate and rate transitions

Format isolation is strict, but transition freshness is deliberately
best-effort. The implementation does not attach generation numbers to packets
and does not promise a hard flush barrier. A bounded number of committed or
in-flight packets can survive if both control changes complete before the
endpoint tasks observe the intermediate mismatch, or if the host later returns
to the same format. That stale data is accepted for this loopback. Packets are
never transferred to a different format's queue, so a transition can never
reinterpret one PCM width as another.

`zerocopy_channel::clear()` is not used. Although `clear()` itself is
synchronous, a sender or receiver can hold a mutable slot grant across an
endpoint `await`. Resetting channel indices from the other half while such a
grant exists does not provide the quiescence protocol needed here. The receiver
instead drains committed packets with `receive_done()`, while uncommitted sender
grants are simply not published.

Endpoint `read` and `write` futures are also not cancelled on a control-state
change. The generic Embassy USB endpoint traits do not specify the logical
state left by cancellation. Each persistent endpoint task lets an in-flight
operation finish, then rechecks the selected alternate/rate before publishing or
continuing. This intentionally permits a boundary packet rather than depending
on driver-specific cancellation behavior.

Loopback is enabled only while both directions have the same nonzero alternate
setting and rate. During the usual host sequence, the host closes a stream (or
selects alternate 0), changes controls, and reopens it. UAC1 also permits less
orderly active-stream Sampling Frequency Control changes; this firmware does
not implement a transactional protocol for that unusual sequence. Unsupported
rates and rates not advertised by the addressed endpoint are rejected.

The class-specific endpoint descriptor advertises Sampling Frequency Control.
`SET_CUR` changes the per-direction rate state, while `GET_CUR`, `GET_MIN`,
`GET_MAX`, and `GET_RES` return three-byte little-endian frequency values. Rate
state is not stored independently for every inactive endpoint; the conventional
close/configure/reopen host sequence is the compatibility target.

## Alternative considered: one endpoint per rate

A simpler state model is possible by exposing only 24-bit PCM and allocating a
separate alternate, endpoint pair, and channel for every sample rate. Then PCM
width, rate, MPS, and queue would all be fixed together, and stale data could be
tolerated without clearing. This design was not adopted because dropping 16-bit
and 32-bit support is a user-visible regression, and rate-per-alternate endpoint
layouts are less common and may have host compatibility costs. The current
per-format design is retained unless those tradeoffs become worthwhile.

## Verification policy

Tests cover state behavior and fractional silence cadence. Tests that merely
repeat constants, descriptor mappings, or round-trip a closed enum are omitted;
the values are derived from `PCM_FORMATS` instead. A mock USB driver also cannot
establish real isochronous timing or host interoperability. Builds, checks, and
Clippy are the routine verification, while probe/real-device tests are useful
when the attached probe is healthy but are not treated as exhaustive models of
host behavior.

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

The AudioControl topology presents line-level playback and capture paths. Both
directions deliberately use the UAC1 Line Connector terminal type. For Capture,
this avoids the positive software gain range that Windows assigns to microphone
endpoints and that can clip the already full-scale loopback signal. For
Playback, Line Connector makes the loopback less likely than a speaker endpoint
to be selected automatically as the default render device. These classifications
affect host policy only; the firmware still transfers digital PCM unchanged.
There is no USB serial string.

Every advertised `(PCM format, sample rate)` combination has an independent
packet queue. After an OUT read completes, the payload is queued under that
physical endpoint's current rate. An IN endpoint reads only the queue for its
own format and current rate. The firmware does not resample, split, merge, or
pad normal loopback packets. If that queue is empty, IN sends digital silence.
It never reinterprets bytes from one PCM width or rate as another.

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

## Endpoint tasks and packet queues

Every nonzero format alternate has its own OUT endpoint task, IN endpoint task,
and physical endpoint. A task's PCM width, frame size, and MPS are fixed when it
is spawned. This prevents an inactive endpoint task from using another format's
larger MPS. Such a mismatch can make an endpoint return `BufferOverflow`
immediately and create an executor-starving retry loop.

Task startup resolves the format slot into the actual `PcmFormat`, endpoint rate
Watch or receiver, and the queue slice for that format. Endpoint tasks retain
those references directly and do not use a global slot to look resources up.

A single dynamic dispatcher is intentionally not used. Persistent per-endpoint
tasks fit Embassy's execution model, and RP2350 has ample RAM for them. The ten
advertised format/rate combinations map to ten bounded `AudioQueue` instances.
Each queue wraps a `heapless::Deque` in a `ThreadModeMutex<RefCell<_>>` and
stores its immutable sample rate alongside it. It exposes only synchronous
`push`, `pop`, and `clear` operations, and a format-specific queue slice can
therefore select a queue by rate without consulting `PcmFormat`. The mapping and
queue count are derived from `PCM_FORMATS`.

Both endpoint tasks run on the same core 0 thread-mode executor, and USB
interrupts only wake endpoint futures; no interrupt handler or RP2350 core 1
accesses queue state. `ThreadModeMutex` relies on this single-core restriction
and does not provide cross-core exclusion. If a queue is later accessed from an
interrupt or core 1, its synchronization must be replaced.

The packet payloads live in a static `heapless::Box` pool containing 100
`BoxBlock<AudioPacket>` blocks. `AudioPacket` remains a `heapless::Vec` with a
fixed 582-byte capacity and runtime payload length. OUT obtains a block and
reads directly into it before observing the rate that won the control/read
ordering and choosing a destination queue. The queue and IN task then transfer
only the pointer-sized `Box` handle; the payload remains in the same block until
IN completes or the packet is discarded. Dropping a packet returns its block to
the pool.

Each format/rate queue has eight handle slots. Capacity does not add latency
while OUT and IN advance at the same rate; normal depth is determined by their
frame phase. Eight slots bound the backlog left by a short interval in which IN
stops while OUT continues. All ten queues can hold 80 packets. The 100-block
pool additionally covers packets held by endpoint operations and leaves spare
capacity for future changes. Its blocks reserve approximately 58 KiB; the
queues themselves store only handles. Pool exhaustion is an internal invariant
failure.

Isochronous OUT cannot be backpressured and retried. `AudioQueue::push` removes
exactly the oldest packet when the queue is full, then inserts the newly received
packet and sets a sticky loss flag within one synchronous borrow. The queue
therefore retains the newest eight packets, although up to seven packets of
latency can remain after IN resumes. An enabled IN task consumes the flag when it
next reads that queue and reports the loss to the LED diagnostic. Clearing a
queue also clears its flag, so overflows accumulated before that IN selection do
not produce a stale diagnostic. USB `BufferOverflow` is treated as a broken
internal MPS invariant and the affected task exits.

## Alternate and rate transitions

Each physical endpoint has its own `Watch` containing either `Unset` or its
configured rate. The watches are independent; there is no duplex snapshot or
stored active alternate. `SET_CUR` updates only the addressed endpoint. An
unconfigured OUT endpoint services and discards packets, while an unconfigured
IN endpoint sends ZLPs.

Capture observes its endpoint Watch generation. On any notification it resets
the silence clock and, if a rate is configured, clears only the queue for the
new `(format, rate)`. Other rate queues remain untouched and are cleared when
they are later selected. Selecting a nonzero alternate re-sends that endpoint's
current Watch value, so reopening at the same rate also triggers this local
clear. The queues own their values, making `clear()` safe even while an
IN task owns a packet it already removed.

Endpoint `read` and `write` futures are also not cancelled on a control-state
change. The generic Embassy USB endpoint traits do not specify the logical
state left by cancellation. Every transfer begins with `wait_enabled()`, but an
in-flight operation is allowed to finish. OUT chooses its destination using the
endpoint rate observed after the read. IN applies a Watch notification before
its next transfer. A packet already removed by IN, or published by a completing
OUT read immediately after a clear, can therefore cross the transition as the
accepted best-effort boundary packet.

A supported reconfiguration must quiesce the affected isochronous submissions
before `SET_CUR` and resume them only after the control sequence is complete.
Passing through alternate setting 0 is the cleanest sequence but is not
required. A host may switch directly between nonzero alternates, or leave the
same nonzero alternate selected while its URBs are stopped and change that
endpoint's rate with `SET_CUR`. The device cannot observe the host's URB queue
directly, so quiescence is a host-side requirement rather than a state the
firmware verifies.

These paths are best-effort rather than transactional. The firmware does not
cancel endpoint I/O or clear controller DPRAM, so a packet already owned by an
endpoint task or controller buffer may cross the transition. That boundary
packet is accepted, but streaming should resume using the newly selected
format and rate. A host that continues submitting isochronous transfers across
`SET_CUR` is explicitly unsupported. Requiring the transfers to stop still
leaves room for a future controller-specific abort and DPRAM flush to provide a
strict boundary without also supporting live-stream `SET_CUR`. Unsupported
rates and rates not advertised by the addressed endpoint are rejected.

The class-specific endpoint descriptor advertises Sampling Frequency Control.
`SET_CUR` changes the addressed physical endpoint's Watch. `GET_CUR` returns its
current value; if it is `Unset`, `GET_CUR` atomically establishes and returns the
format default (currently 48 kHz). `GET_MIN`, `GET_MAX`, and `GET_RES` return
three-byte little-endian frequency values without changing state. Inactive
endpoints retain independent settings. Both the conventional alternate-0
close/configure/reopen sequence and stopped-URB reconfiguration without an
explicit alternate-0 transition are compatibility targets.

## Alternative considered: one endpoint per rate

A simpler state model is possible by exposing only 24-bit PCM and allocating a
separate alternate, endpoint pair, and channel for every sample rate. Then PCM
width, rate, MPS, and queue would all be fixed together, and stale data could be
tolerated without clearing. This design was not adopted because dropping 16-bit
and 32-bit support is a user-visible regression, and rate-per-alternate endpoint
layouts are less common and may have host compatibility costs. The current
per-format design is retained unless those tradeoffs become worthwhile.

## Verification policy

Tests cover independent endpoint state, Watch notifications, format/rate queue
mapping, drop-oldest behavior, and fractional silence cadence. A mock USB driver
still cannot establish real isochronous timing or host interoperability. Builds,
checks, and Clippy are the routine verification, while probe/real-device tests
are useful when the attached probe is healthy but are not treated as exhaustive
models of host behavior. Real-host smoke testing should include, when the host
exposes it, a Windows-style reconfiguration that stops URBs without explicitly
selecting alternate 0. The check is only that streaming resumes at the selected
format and rate; the accepted boundary packet is not treated as a failure, and
continuous-traffic `SET_CUR` is not a supported test case.

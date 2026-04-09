# CLAUDE.md — bilbycast-fdk-aac-rs

## What Is This

Rust wrapper around Fraunhofer FDK AAC v2.0.3 for the bilbycast ecosystem. Provides safe, in-process AAC decoding and encoding — replacing symphonia (decode) and ffmpeg subprocess (encode) in bilbycast-edge.

## Projects

| Crate | Role |
|-------|------|
| **libfdk-aac-sys** | Raw FFI bindings to fdk-aac via bindgen. Vendored build from `vendor/fdk-aac/` (git submodule). |
| **aac-codec** | Pure-Rust data types (config, errors, stream info). No C dependency. |
| **aac-audio** | Safe wrapper — `AacDecoder` and `AacEncoder`. The crate bilbycast-edge depends on. |

## Codec Support

| Feature | Decode | Encode |
|---------|--------|--------|
| AAC-LC | Yes | Yes |
| HE-AAC v1 (SBR) | Yes | Yes |
| HE-AAC v2 (PS) | Yes | Yes (stereo only) |
| AAC-LD | Yes | Yes |
| AAC-ELD | Yes | Yes |
| Multichannel (up to 7.1) | Yes | Yes |
| ADTS framing | Yes | Yes (output) |
| LATM framing | Yes | No |
| Raw access units | Yes | Yes (output) |

## Build & Test

```bash
# Build all crates (requires CMake for vendored fdk-aac build)
cargo build

# Run tests
cargo test

# Use system libfdk-aac instead of vendored
cargo build --features libfdk-aac-sys/system-libfdk-aac

# Point to custom fdk-aac install
LIBFDK_AAC_DIR=/path/to/fdk-aac cargo build
```

### Prerequisites

- **CMake** (for vendored fdk-aac build)
- **Clang/LLVM** (for bindgen)
- **macOS**: `brew install cmake`
- **Linux**: `apt install cmake clang`

No OpenSSL required (unlike bilbycast-libsrt-rs).

## Architecture

### Decoder

`AacDecoder` wraps the fdk-aac `aacDecoder_*` API:
- `open_adts()` — for complete ADTS frames (including header)
- `open_latm()` — for LATM/LOAS framing
- `open_raw(asc)` — for raw access units with AudioSpecificConfig (used by bilbycast-edge since its demuxer strips ADTS headers)
- `decode_frame(data)` → `DecodedFrame { planar: Vec<Vec<f32>>, frame_size }`
- Output is planar f32 PCM matching bilbycast-edge's existing audio pipeline API
- Internal: fdk-aac outputs interleaved INT_PCM (s16), wrapper deinterleaves to planar f32

### Encoder

`AacEncoder` wraps the fdk-aac `aacEncoder_*` API:
- `open(config)` — configure profile, sample rate, channels, bitrate, transport
- `encode_frame(planar)` — encode planar f32 PCM to AAC bitstream
- `encode_frame_s16(interleaved)` — encode from interleaved s16 directly (avoids conversion)
- `flush()` — end-of-stream flush
- `audio_specific_config()` — for FLV/SDP signaling
- Internal: wrapper converts planar f32 to interleaved s16, sets up AACENC_BufDesc, calls aacEncEncode

### Helper

`build_audio_specific_config(profile, sample_rate_index, channel_config)` — constructs a 2-byte ASC from ADTS header fields, for use with `AacDecoder::open_raw()`.

## Key Design Constraints

1. **Per-instance state only** — no global init/cleanup (unlike libsrt)
2. **Send but not Sync** — handles can move between threads but require &mut for decode/encode
3. **INT_PCM is s16** — fdk-aac compiled with default SYS_S16; buffer sizing assumes this
4. **Frame sizes vary by profile** — AAC-LC: 1024, HE-AAC: 2048, LD/ELD: 480/512. Use `frame_size()` accessor
5. **HE-AAC v2 requires stereo** — mono input rejected at open time

## Integration with bilbycast-edge

bilbycast-edge's `TsDemuxer` strips ADTS headers and caches `(profile, sample_rate_index, channel_config)`. Use `build_audio_specific_config()` to construct the ASC, then `AacDecoder::open_raw(&asc)` to create the decoder. Feed raw AAC access units via `decode_frame()`.

For encoding, use `EncoderConfig::aac_lc()` / `he_aac_v1()` / `he_aac_v2()` constructors, then `AacEncoder::open(&config)`. Feed planar f32 PCM from the decode stage.

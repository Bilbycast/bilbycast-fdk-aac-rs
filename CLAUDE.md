# CLAUDE.md — bilbycast-fdk-aac-rs

## What Is This

Rust wrapper around Fraunhofer FDK AAC v2.0.3 for the bilbycast ecosystem. Provides safe, in-process AAC decoding and encoding — replacing symphonia (decode) and ffmpeg subprocess (encode) in bilbycast-edge.

## Projects

| Crate | Role |
|-------|------|
| **libfdk-aac-sys** | Raw FFI bindings to fdk-aac via bindgen. Vendored build from `libfdk-aac-sys/vendor/fdk-aac` (git submodule). |
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
| LATM framing | Yes | Yes (set `EncoderConfig.transport = TransportType::Latm`; the convenience constructors default to ADTS) |
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
- `output_delay_samples()` / `StreamInfo.output_delay` — fdk-aac's `CStreamInfo.outputDelay`, `None` until the first decode

**Every constructor changes three fdk-aac defaults** (`open_internal`, which closes the handle if any `aacDecoder_SetParam` fails and returns `AacError::DecoderSetParam`):

| Setting | fdk-aac default | Here | Why |
|---|---|---|---|
| `AAC_CONCEAL_METHOD` | 2, energy interpolation (holds one frame back) | 1, noise substitution | No delay. The wrapper never passes `AACDEC_CONCEAL` and returns `Err` (PCM discarded) on a decode error, so the method has no audible effect here — the removed frame of delay is the whole change |
| `AAC_PCM_LIMITER_ENABLE` | auto: on for every non-LD/ELD AOT, 15 ms attack lookahead | 0, off | No delay. Cost: a sample reconstructed past full scale hard-clips at the s16 conversion instead of being soft-limited |
| `AAC_DRC_REFERENCE_LEVEL` | 96, i.e. normalise to -24 dB | -1, off | A stream carrying MPEG-4 `prog_ref_level` was re-levelled by `-24 - prog_ref_level` dB: -31 dB boosted +7 dB (which hard-clips with the limiter off), -18 dB cut 6 dB. The metadata does not survive a re-encode, so the gain was baked in and biased loudness meters. Off, PCM is at the encoded level, as from libavcodec's AC-3 / E-AC-3 decoders (`target_level` 0) |

Left at the defaults, AAC-LC decode was **1744 samples late at 48 kHz** (1024 concealment + 720 limiter; 1685 at 44.1 kHz), i.e. 36.3 ms that no consumer compensated. Now `output_delay` is **0 for AAC-LC / LD / ELD**, and an `AacEncoder` → `AacDecoder` round trip lands content exactly `AacEncoder::codec_delay_samples()` late (2048 for AAC-LC). For **HE-AAC v1/v2 it is 962** (the SBR QMF delay at the output rate; 481 downsampled): part of the codec, cannot be switched off, and fdk-aac's encoder already counts it in its `nDelay` (5058 for stereo HE-AAC v1 at 48 kHz), so a chain that stamps `input_pts - nDelay` must **not** subtract it again. A stream stamped the other way (edit list from `nDelayCore`, which excludes it) expects the decoder to remove it, as FFmpeg's `libfdk_aac` decoder does. Pinned by `aac-audio/tests/decoder_delay.rs` (delay + burst alignment, 48 / 44.1 kHz, ADTS + raw, in-band reconfig, HE-AAC) and `aac-audio/tests/decoder_level.rs` (streams encoded with `prog_ref_level` -31 / -18 dB through the raw FFI, which is why `AACENC_MetaData` is on the bindgen allowlist).

### Encoder

`AacEncoder` wraps the fdk-aac `aacEncoder_*` API:
- `open(config)` — configure profile, sample rate, channels, bitrate, transport
- `encode_frame(planar)` — encode planar f32 PCM to AAC bitstream
- `encode_frame_s16(interleaved)` — encode from interleaved s16 directly (avoids conversion)
- `flush()` — end-of-stream flush
- `audio_specific_config()` — for FLV/SDP signaling
- `codec_delay_samples()` — `AACENC_InfoStruct.nDelay`, to subtract from input-derived output timestamps: **2048 for AAC-LC** (it follows the frame length, not the rate: `DELAY_AAC` 1600 = 1024 MDCT + 576 block-switch lookahead, plus the metadata module's 448 round-up; pinned by `aac_lc_encoder_delay_is_2048`), more for HE-AAC because it includes the decoder's SBR delay. Older docs said "≈ 2600 / 54 ms"; that figure was never right
- `EncoderConfig.transport` picks the transmux and `configure()` passes it straight to `AACENC_TRANSMUX`: `Raw` → 0, `Adts` → 2, `Latm` → 10. `open()` rejects none of them, so **LATM output is available on the encoder**, not decode-only. It is easy to believe otherwise because all three convenience constructors (`aac_lc` / `he_aac_v1` / `he_aac_v2`) hard-code `transport: TransportType::Adts` — LATM means setting the field yourself after building the config. (Value 10 is fdk-aac's LOAS audio-sync-stream transmux, the self-framing LATM carriage a TS/RTP consumer wants; the `// TT_MP4_LATM_MCP1` comment beside it in `encoder.rs` is mislabelled — MCP1 is 6 in the vendored `FDK_audio.h`.)
- Internal: wrapper converts planar f32 to interleaved s16, sets up AACENC_BufDesc, calls aacEncEncode

### Helper

`build_audio_specific_config(profile, sample_rate_index, channel_config)` — constructs a 2-byte ASC from ADTS header fields, for use with `AacDecoder::open_raw()`.

## Key Design Constraints

1. **Per-instance state only** — no global init/cleanup (unlike libsrt)
2. **Send but not Sync** — handles can move between threads but require &mut for decode/encode
3. **INT_PCM is s16** — fdk-aac compiled with default SYS_S16; buffer sizing assumes this
4. **Frame sizes vary by profile** — AAC-LC: 1024, HE-AAC: 2048, LD/ELD: 480/512. Use the `AacEncoder::frame_size()` accessor (encoder only); the decoder reports it as the `frame_size` field on each `DecodedFrame`
5. **HE-AAC v2 requires stereo** — mono input rejected at open time

## Integration with bilbycast-edge

bilbycast-edge's `TsDemuxer` strips ADTS headers and caches `(profile, sample_rate_index, channel_config)`. Use `build_audio_specific_config()` to construct the ASC, then `AacDecoder::open_raw(&asc)` to create the decoder. Feed raw AAC access units via `decode_frame()`.

For encoding, use `EncoderConfig::aac_lc()` / `he_aac_v1()` / `he_aac_v2()` constructors, then `AacEncoder::open(&config)`. Feed planar f32 PCM from the decode stage.

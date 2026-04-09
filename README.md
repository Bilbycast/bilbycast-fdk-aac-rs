# bilbycast-fdk-aac-rs

Rust wrapper around [Fraunhofer FDK AAC](https://github.com/mstorsjo/fdk-aac) v2.0.3, providing safe, in-process AAC decoding and encoding for the [bilbycast](https://github.com/softside-tech/bilbycast) broadcast media transport ecosystem.

## What It Does

bilbycast-fdk-aac-rs enables bilbycast-edge to decode and encode AAC audio without relying on external processes. It replaces the pure-Rust `symphonia` decoder (limited to AAC-LC mono/stereo) and the `ffmpeg` subprocess encoder with a single, high-quality in-process codec library.

The marquee use case is **AAC contribution audio in → broadcast distribution out**: AAC streams arriving via RTMP, RTSP, SRT, or RTP are decoded to PCM, then either forwarded as uncompressed audio (SMPTE ST 2110-30, SMPTE 302M) or re-encoded into AAC, HE-AAC, Opus, or other codecs for WebRTC, HLS, and RTMP outputs.

## Workspace Crates

| Crate | Role |
|-------|------|
| **libfdk-aac-sys** | Raw FFI bindings via `bindgen`. Builds fdk-aac from a vendored git submodule (`vendor/fdk-aac/`) using CMake. |
| **aac-codec** | Pure-Rust data types — codec configuration, error types, stream info. No C dependency. |
| **aac-audio** | Safe high-level API — `AacDecoder` and `AacEncoder`. This is the crate that bilbycast-edge depends on. |

## Codec Support

| Profile | Decode | Encode |
|---------|--------|--------|
| AAC-LC | Yes | Yes |
| HE-AAC v1 (SBR) | Yes | Yes |
| HE-AAC v2 (PS) | Yes | Yes (stereo only) |
| AAC-LD | Yes | Yes |
| AAC-ELD | Yes | Yes |
| Multichannel (up to 7.1) | Yes | Yes |

Supported input framing: ADTS, LATM/LOAS, and raw access units (with AudioSpecificConfig).

## Building

### Prerequisites

- **CMake** — for the vendored fdk-aac C build
- **Clang/LLVM** — for `bindgen` FFI generation
- macOS: `brew install cmake`
- Linux: `apt install cmake clang`

### Build & Test

```bash
cargo build
cargo test
```

To use a system-installed libfdk-aac instead of the vendored copy:

```bash
cargo build --features libfdk-aac-sys/system-libfdk-aac
```

## Usage in bilbycast-edge

This crate is enabled via the `fdk-aac` feature flag in bilbycast-edge (on by default). When enabled, all AAC decode and encode operations use FDK AAC in-process. When disabled, bilbycast-edge falls back to `symphonia` for decoding (AAC-LC only) and `ffmpeg` subprocess for encoding.

## License

This project is licensed under the [Mozilla Public License 2.0](LICENSE).

Note: The vendored Fraunhofer FDK AAC library (`vendor/fdk-aac/`) is licensed under its own terms — see the license file within that submodule for details.

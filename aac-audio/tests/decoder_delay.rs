// Copyright (c) 2026 Softside Tech Pty Ltd. All rights reserved.
// SPDX-License-Identifier: MPL-2.0

//! The decoder must add no delay of its own: an AAC round trip through this
//! crate lands content exactly the encoder's declared delay
//! (`AacEncoder::codec_delay_samples`) later, and nothing more.
//!
//! fdk-aac's decoder defaults add 1744 samples at 48 kHz on AAC-LC (a frame
//! of energy-interpolation concealment plus 720 samples of limiter
//! lookahead); `AacDecoder` switches both off at open. These tests fail
//! against a decoder opened with the library defaults.

use aac_audio::{AacDecoder, AacEncoder, EncoderConfig, TransportType};

/// Hann-windowed tone burst: band-limited, so the MDCT reproduces it without
/// the pre-echo that would smear a one-sample click across a block.
fn burst(len: usize, freq_hz: f32, sample_rate: u32) -> Vec<f32> {
    (0..len)
        .map(|i| {
            let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (len - 1) as f32).cos();
            let t = i as f32 / sample_rate as f32;
            0.5 * w * (2.0 * std::f32::consts::PI * freq_hz * t).sin()
        })
        .collect()
}

/// Encode `signal` (mono content, copied to every channel) frame by frame and
/// return one access unit per input frame. Asserts every call emits an AU,
/// so AU `k` carries the encoder output for input frame `k`.
fn encode(enc: &mut AacEncoder, signal: &[f32]) -> Vec<Vec<u8>> {
    let fs = enc.frame_size() as usize;
    let ch = enc.channels() as usize;
    assert_eq!(signal.len() % fs, 0, "signal must be a whole number of frames");
    signal
        .chunks(fs)
        .enumerate()
        .map(|(k, chunk)| {
            let planar = vec![chunk.to_vec(); ch];
            let out = enc.encode_frame(&planar).expect("encode");
            assert!(!out.bytes.is_empty(), "encoder emitted no AU for input frame {k}");
            out.bytes
        })
        .collect()
}

/// Decode `aus` in order and return channel 0 of every decoded frame,
/// concatenated.
fn decode(dec: &mut AacDecoder, aus: &[Vec<u8>]) -> Vec<f32> {
    let mut pcm = Vec::new();
    for (k, au) in aus.iter().enumerate() {
        let frame = dec
            .decode_frame(au)
            .unwrap_or_else(|e| panic!("decode of AU {k} failed: {e}"));
        pcm.extend_from_slice(&frame.planar[0]);
    }
    pcm
}

/// Index in `haystack` at which `needle` correlates best.
fn locate(haystack: &[f32], needle: &[f32]) -> usize {
    let mut best = (0usize, f32::MIN);
    for lag in 0..=haystack.len() - needle.len() {
        let c: f32 = haystack[lag..lag + needle.len()]
            .iter()
            .zip(needle)
            .map(|(a, b)| a * b)
            .sum();
        if c > best.1 {
            best = (lag, c);
        }
    }
    best.0
}

/// A burst placed at `at` in `frames` frames of silence.
fn signal_with_burst(frames: usize, frame_size: usize, at: usize, b: &[f32]) -> Vec<f32> {
    let mut s = vec![0.0f32; frames * frame_size];
    s[at..at + b.len()].copy_from_slice(b);
    s
}

/// Round-trip a burst through `cfg` and return (landing index − input index,
/// encoder-declared delay, decoder-reported output delay).
fn round_trip(cfg: &EncoderConfig, open: impl Fn(&AacEncoder) -> AacDecoder) -> (usize, u32, u32) {
    let mut enc = AacEncoder::open(cfg).expect("open encoder");
    let fs = enc.frame_size() as usize;
    let b = burst(256, 2000.0, cfg.sample_rate);
    // Well clear of the first frames and of the end, whatever the delay.
    let at = 3 * fs + 100;
    let signal = signal_with_burst(12, fs, at, &b);
    let aus = encode(&mut enc, &signal);

    let mut dec = open(&enc);
    let pcm = decode(&mut dec, &aus);
    assert_eq!(pcm.len(), signal.len(), "one decoded frame per AU");

    let landed = locate(&pcm, &b);
    let out_delay = dec.output_delay_samples().expect("stream info after a decode");
    assert_eq!(dec.stream_info().unwrap().output_delay, out_delay);
    let lag = landed
        .checked_sub(at)
        .unwrap_or_else(|| panic!("burst found at {landed}, before its input position {at}"));
    (lag, enc.codec_delay_samples(), out_delay)
}

fn assert_within_one(actual: usize, expected: u32, what: &str) {
    let d = actual as i64 - expected as i64;
    assert!(d.abs() <= 1, "{what}: content landed {actual} samples late, expected {expected} ±1");
}

#[test]
fn aac_lc_adts_48k_decoder_adds_no_delay() {
    let cfg = EncoderConfig::aac_lc(48_000, 2, 192_000);
    let (lag, enc_delay, out_delay) = round_trip(&cfg, |_| AacDecoder::open_adts().unwrap());
    // The reported figure describes the PCM, whatever it is...
    assert_within_one(lag, enc_delay + out_delay, "encoder delay + reported output delay");
    // ...and the encoder's delay is the whole round trip: the first decoded
    // sample is AU 0's first sample, with no 1744-sample lag.
    assert_within_one(lag, enc_delay, "AAC-LC 48 kHz ADTS");
    assert_eq!(out_delay, 0, "decoder output delay (library default: 1024 + 720 = 1744)");
}

#[test]
fn aac_lc_adts_44k1_decoder_adds_no_delay() {
    // The limiter lookahead scales with the rate (661 at 44.1 kHz), so this
    // is not the 48 kHz case again.
    let cfg = EncoderConfig::aac_lc(44_100, 2, 192_000);
    let (lag, enc_delay, out_delay) = round_trip(&cfg, |_| AacDecoder::open_adts().unwrap());
    assert_within_one(lag, enc_delay + out_delay, "encoder delay + reported output delay");
    assert_within_one(lag, enc_delay, "AAC-LC 44.1 kHz ADTS");
    assert_eq!(out_delay, 0, "decoder output delay (library default: 1024 + 661 = 1685)");
}

#[test]
fn aac_lc_raw_decoder_adds_no_delay() {
    // `open_raw` is the path bilbycast-edge's demuxer uses: the settings must
    // survive the `ConfigRaw` that follows the open.
    let mut cfg = EncoderConfig::aac_lc(48_000, 2, 192_000);
    cfg.transport = TransportType::Raw;
    let (lag, enc_delay, out_delay) = round_trip(&cfg, |enc| {
        AacDecoder::open_raw(enc.audio_specific_config()).expect("open_raw")
    });
    assert_within_one(lag, enc_delay + out_delay, "encoder delay + reported output delay");
    assert_within_one(lag, enc_delay, "AAC-LC 48 kHz raw");
    assert_eq!(out_delay, 0, "decoder output delay");
}

#[test]
fn aac_lc_encoder_delay_is_2048() {
    // DELAY_AAC(1024) = 1024 + 4*128 + 64 = 1600, plus the metadata
    // module's 448-sample round-up; it follows the frame length, not the
    // rate. Pinned because callers budget for it and the old docs said
    // ~2600.
    for rate in [48_000, 44_100] {
        let enc = AacEncoder::open(&EncoderConfig::aac_lc(rate, 2, 192_000)).unwrap();
        assert_eq!(enc.codec_delay_samples(), 2048, "AAC-LC nDelay at {rate} Hz");
    }
}

#[test]
fn settings_survive_in_band_reconfiguration() {
    // An ADTS stream that changes rate mid-stream reconfigures the decoder
    // in band; the no-delay settings must still hold afterwards.
    let mut dec = AacDecoder::open_adts().unwrap();
    for rate in [48_000u32, 44_100] {
        let mut enc = AacEncoder::open(&EncoderConfig::aac_lc(rate, 2, 192_000)).unwrap();
        let silence = vec![0.0f32; 4 * enc.frame_size() as usize];
        let aus = encode(&mut enc, &silence);
        decode(&mut dec, &aus);
        assert_eq!(dec.sample_rate(), Some(rate));
        assert_eq!(dec.output_delay_samples(), Some(0), "output delay after switching to {rate} Hz");
    }
}

#[test]
fn he_aac_v1_output_delay_is_only_the_sbr_delay() {
    // What remains on an SBR stream is the SBR QMF delay (962 at the output
    // rate), which fdk-aac's encoder already counts in its nDelay. The pair
    // is therefore self-consistent: the round trip equals nDelay exactly.
    let cfg = EncoderConfig::he_aac_v1(48_000, 2, 64_000);
    let (lag, enc_delay, out_delay) = round_trip(&cfg, |_| AacDecoder::open_adts().unwrap());
    assert_within_one(lag, enc_delay, "HE-AAC v1 48 kHz ADTS");
    assert_eq!(out_delay, 962, "SBR QMF delay, with no concealment or limiter delay on top");
}

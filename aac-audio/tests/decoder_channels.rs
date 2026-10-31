// Copyright (c) 2026 Softside Tech Pty Ltd. All rights reserved.
// SPDX-License-Identifier: MPL-2.0

//! How many channels the decoder hands out. fdk-aac mixes anything wider
//! than six channels down to 5.1 unless told otherwise, so a 7.1 stream
//! decoded as six; `AacDecoder::set_max_output_channels(None)` lets all
//! eight through, each its own.

use aac_audio::{AacDecoder, AacEncoder, AacProfile, EncoderConfig, SbrSignaling, TransportType};

const RATE: u32 = 48_000;

/// The tone input channel `ch` carries.
fn tone_hz(ch: usize) -> f64 {
    300.0 + 200.0 * ch as f64
}

/// Two seconds of 7.1 AAC-LC as raw access units, a distinct tone on each
/// input channel (WAV order), and the encoder's AudioSpecificConfig
/// (channel configuration 12, which an ASC's four bits can hold).
fn seven_one() -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut enc = AacEncoder::open(&EncoderConfig {
        profile: AacProfile::AacLc,
        sample_rate: RATE,
        channels: 8,
        bitrate: 448_000,
        afterburner: true,
        sbr_signaling: SbrSignaling::default(),
        transport: TransportType::Raw,
    })
    .expect("a 7.1 encoder");
    let n = enc.frame_size() as usize;
    let mut aus = Vec::new();
    for k in 0..(2 * RATE as usize / n) {
        let planar: Vec<Vec<f32>> = (0..8)
            .map(|ch| {
                (0..n)
                    .map(|i| {
                        let t = (k * n + i) as f64 / f64::from(RATE);
                        (0.2 * (2.0 * std::f64::consts::PI * tone_hz(ch) * t).sin()) as f32
                    })
                    .collect()
            })
            .collect();
        let out = enc.encode_frame(&planar).expect("encode");
        if !out.bytes.is_empty() {
            aus.push(out.bytes);
        }
    }
    (enc.audio_specific_config().to_vec(), aus)
}

/// Decode `aus`, past the first four frames (priming), and return for each
/// output channel the input channel whose tone is loudest on it, with the
/// number of channels every frame carried.
fn loudest_inputs(dec: &mut AacDecoder, aus: &[Vec<u8>]) -> (usize, Vec<usize>) {
    let mut power: Vec<[f64; 8]> = Vec::new();
    let mut channels = None;
    for (k, au) in aus.iter().enumerate() {
        let frame = dec.decode_frame(au).unwrap_or_else(|e| panic!("AU {k}: {e}"));
        let ch = frame.planar.len();
        assert_eq!(*channels.get_or_insert(ch), ch, "AU {k}: the channel count changed");
        power.resize(ch, [0.0; 8]);
        if k < 4 {
            continue;
        }
        for (out, samples) in frame.planar.iter().enumerate() {
            for (input, p) in power[out].iter_mut().enumerate() {
                // Goertzel at input `input`'s tone.
                let c = 2.0 * (2.0 * std::f64::consts::PI * tone_hz(input) / f64::from(RATE)).cos();
                let (mut s1, mut s2) = (0.0f64, 0.0f64);
                for &x in samples {
                    let s0 = f64::from(x) + c * s1 - s2;
                    s2 = s1;
                    s1 = s0;
                }
                *p += s1 * s1 + s2 * s2 - c * s1 * s2;
            }
        }
    }
    let loudest = power.iter().map(|p| (0..8).max_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap()).collect();
    (channels.expect("no frame"), loudest)
}

/// fdk-aac's default: 7.1 comes out mixed down to six channels.
#[test]
fn a_7_1_stream_decodes_to_six_channels_by_default() {
    let (asc, aus) = seven_one();
    let mut dec = AacDecoder::open_raw(&asc).expect("decoder");
    let (channels, _) = loudest_inputs(&mut dec, &aus);
    assert_eq!(channels, 6);
    assert_eq!(dec.channels(), Some(6));
}

/// With no limit, all eight channels: each input's tone loudest on a channel
/// of its own (the LFE's 900 Hz is filtered out by the encoder's LFE band
/// limit, so seven inputs are told apart and the LFE is the one left over).
#[test]
fn with_no_limit_a_7_1_stream_decodes_to_eight_channels() {
    let (asc, aus) = seven_one();
    let mut dec = AacDecoder::open_raw(&asc).expect("decoder");
    dec.set_max_output_channels(None).expect("no limit");
    let (channels, loudest) = loudest_inputs(&mut dec, &aus);
    assert_eq!(channels, 8);
    assert_eq!(dec.channels(), Some(8));
    assert_eq!(dec.stream_info().map(|si| si.sample_rate), Some(RATE));
    let mut heard: Vec<usize> = loudest.iter().copied().filter(|&input| input != 3).collect();
    heard.sort_unstable();
    heard.dedup();
    assert_eq!(heard, [0, 1, 2, 4, 5, 6, 7], "each input but the LFE on its own channel: {loudest:?}");
}

/// A cap below the stream's width mixes down to it; a cap the decoder has no
/// meaning for is refused.
#[test]
fn a_cap_mixes_down_and_a_bad_one_is_refused() {
    let (asc, aus) = seven_one();
    let mut dec = AacDecoder::open_raw(&asc).expect("decoder");
    dec.set_max_output_channels(Some(2)).expect("stereo");
    let (channels, _) = loudest_inputs(&mut dec, &aus);
    assert_eq!(channels, 2);
    let mut dec = AacDecoder::open_adts().expect("decoder");
    assert!(dec.set_max_output_channels(Some(0)).is_err());
    assert!(dec.set_max_output_channels(Some(9)).is_err());
    assert!(dec.set_max_output_channels(Some(8)).is_ok());
}

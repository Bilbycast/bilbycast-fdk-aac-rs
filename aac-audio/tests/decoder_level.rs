// Copyright (c) 2026 Softside Tech Pty Ltd. All rights reserved.
// SPDX-License-Identifier: MPL-2.0

//! The decoder must deliver PCM at the encoded level, whatever loudness
//! metadata the stream carries.
//!
//! fdk-aac's decoder normalises by default: a stream carrying an MPEG-4
//! `prog_ref_level` is re-levelled to a -24 dB target, so -31 dB is boosted
//! 7 dB and -18 dB is cut 6 dB. With the PCM limiter off (which the
//! zero-delay decoder needs), the boost hard-clips. `AacDecoder` switches
//! normalisation off; these tests fail against a decoder that leaves it on.
//!
//! `AacEncoder` does not expose metadata, so the streams are encoded here
//! through the raw FFI with fdk-aac's metadata module.

use aac_audio::AacDecoder;
use libfdk_aac_sys::*;

const RATE: u32 = 48_000;
const FRAME: usize = 1024;
const AMPLITUDE: f32 = 0.5;

/// Stereo AAC-LC ADTS of a 997 Hz sine at `AMPLITUDE`, carrying
/// `dynamic_range_info` with `prog_ref_level = prog_ref_level_db`.
fn encode_with_prog_ref_level(prog_ref_level_db: i32, frames: usize) -> Vec<Vec<u8>> {
    // SAFETY: plain fdk-aac encoder calls on a handle this function owns;
    // every buffer outlives the call that reads it.
    unsafe {
        let mut h: HANDLE_AACENCODER = std::ptr::null_mut();
        assert_eq!(aacEncOpen(&mut h, 0, 2), AACENC_ERROR_AACENC_OK);
        for (param, value) in [
            (AACENC_PARAM_AACENC_AOT, 2),
            (AACENC_PARAM_AACENC_SAMPLERATE, RATE),
            (AACENC_PARAM_AACENC_CHANNELMODE, 2),
            (AACENC_PARAM_AACENC_CHANNELORDER, 1),
            (AACENC_PARAM_AACENC_BITRATE, 256_000),
            (AACENC_PARAM_AACENC_TRANSMUX, 2), // ADTS
            (AACENC_PARAM_AACENC_METADATA_MODE, 1), // dynamic_range_info
        ] {
            assert_eq!(aacEncoder_SetParam(h, param, value), AACENC_ERROR_AACENC_OK);
        }
        let init = aacEncEncode(
            h,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
        );
        assert_eq!(init, AACENC_ERROR_AACENC_OK);

        let mut meta = AACENC_MetaData {
            drc_profile: AACENC_METADATA_DRC_PROFILE_AACENC_METADATA_DRC_NONE,
            comp_profile: AACENC_METADATA_DRC_PROFILE_AACENC_METADATA_DRC_NOT_PRESENT,
            prog_ref_level_present: 1,
            prog_ref_level: prog_ref_level_db << 16, // dB, Q16
            ..Default::default()
        };

        let mut out = vec![0u8; 8192];
        let mut aus = Vec::with_capacity(frames);
        for k in 0..frames {
            let mut pcm = vec![0i16; FRAME * 2];
            for s in 0..FRAME {
                let t = (k * FRAME + s) as f32 / RATE as f32;
                let v = AMPLITUDE * (2.0 * std::f32::consts::PI * 997.0 * t).sin();
                pcm[2 * s] = (v * 32767.0) as i16;
                pcm[2 * s + 1] = pcm[2 * s];
            }
            let mut in_ptrs: [*mut std::ffi::c_void; 2] = [
                pcm.as_mut_ptr().cast(),
                (&mut meta as *mut AACENC_MetaData).cast(),
            ];
            let meta_size = std::mem::size_of::<AACENC_MetaData>() as i32;
            // IN_AUDIO_DATA = 0, IN_METADATA_SETUP = 2.
            let mut in_ids = [0i32, 2];
            let mut in_sizes = [(FRAME * 2 * 2) as i32, meta_size];
            let mut in_el_sizes = [2i32, meta_size];
            let in_desc = AACENC_BufDesc {
                numBufs: 2,
                bufs: in_ptrs.as_mut_ptr(),
                bufferIdentifiers: in_ids.as_mut_ptr(),
                bufSizes: in_sizes.as_mut_ptr(),
                bufElSizes: in_el_sizes.as_mut_ptr(),
            };
            let mut out_ptr: *mut std::ffi::c_void = out.as_mut_ptr().cast();
            let mut out_id = 3i32; // OUT_BITSTREAM_DATA
            let mut out_size = out.len() as i32;
            let mut out_el_size = 1i32;
            let out_desc = AACENC_BufDesc {
                numBufs: 1,
                bufs: &mut out_ptr,
                bufferIdentifiers: &mut out_id,
                bufSizes: &mut out_size,
                bufElSizes: &mut out_el_size,
            };
            let in_args = AACENC_InArgs {
                numInSamples: (FRAME * 2) as i32,
                numAncBytes: 0,
            };
            let mut out_args: AACENC_OutArgs = std::mem::zeroed();
            let err = aacEncEncode(h, &in_desc, &out_desc, &in_args, &mut out_args);
            assert_eq!(err, AACENC_ERROR_AACENC_OK);
            aus.push(out[..out_args.numOutBytes as usize].to_vec());
        }
        aacEncClose(&mut h);
        aus
    }
}

/// Decode and return (peak, samples at full scale) of channel 0, skipping
/// the encoder's start-up delay.
fn decoded_peak(aus: &[Vec<u8>]) -> (f32, usize) {
    let mut dec = AacDecoder::open_adts().unwrap();
    let mut pcm = Vec::new();
    for au in aus {
        pcm.extend_from_slice(&dec.decode_frame(au).expect("decode").planar[0]);
    }
    let settled = &pcm[4 * FRAME..];
    let peak = settled.iter().fold(0f32, |m, x| m.max(x.abs()));
    let clipped = settled.iter().filter(|x| x.abs() >= 32767.0 / 32768.0).count();
    (peak, clipped)
}

#[test]
fn quiet_prog_ref_level_is_not_boosted_into_clipping() {
    // -31 dB declared: fdk-aac's default would add 7 dB, taking the 0.5
    // sine to ~1.1 and, with the limiter off, hard-clipping most samples.
    let (peak, clipped) = decoded_peak(&encode_with_prog_ref_level(-31, 24));
    assert_eq!(clipped, 0, "samples hard-clipped at full scale (peak {peak})");
    assert!((peak - AMPLITUDE).abs() < 0.03, "decoded peak {peak}, encoded {AMPLITUDE}");
}

#[test]
fn loud_prog_ref_level_is_not_cut() {
    // -18 dB declared: fdk-aac's default would take 6 dB off (peak ~0.25).
    let (peak, _) = decoded_peak(&encode_with_prog_ref_level(-18, 24));
    assert!((peak - AMPLITUDE).abs() < 0.03, "decoded peak {peak}, encoded {AMPLITUDE}");
}

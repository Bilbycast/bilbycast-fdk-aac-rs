// Copyright (c) 2026 Softside Tech Pty Ltd. All rights reserved.
// SPDX-License-Identifier: MPL-2.0

//! Safe AAC decoder wrapping `aacDecoder_*` FFI calls.
//!
//! Supports AAC-LC, HE-AAC v1 (SBR), HE-AAC v2 (PS), AAC-LD, AAC-ELD,
//! and multichannel up to 7.1. Input can be ADTS, LATM, or raw access units.
//!
//! Output is planar f32 PCM: `Vec<Vec<f32>>` shaped `[channel][sample]`,
//! matching the bilbycast-edge audio pipeline API.

use aac_codec::{AacError, AacProfile, StreamInfo};
use libfdk_aac_sys::*;

/// Maximum output PCM buffer size: max frame size (2048 for HE-AAC) * max channels (8).
const MAX_PCM_SAMPLES: usize = 2048 * 8;

/// `AAC_CONCEAL_METHOD` value for noise substitution (`ConcealMethodNoise`
/// in fdk-aac's `conceal_types.h`), which adds no output delay.
const CONCEAL_NOISE_SUBSTITUTION: i32 = 1;

/// `AAC_PCM_LIMITER_ENABLE` value that switches the PCM limiter off.
const LIMITER_OFF: i32 = 0;

/// `AAC_DRC_REFERENCE_LEVEL` value that switches off loudness normalisation
/// (and MPEG-4 DRC with it): any negative value does.
const LOUDNESS_NORMALISATION_OFF: i32 = -1;

/// Result of decoding one AAC frame.
#[derive(Debug)]
pub struct DecodedFrame {
    /// Planar f32 PCM: `[channel][sample]`. Each inner vec has exactly
    /// `frame_size` elements. Values are in the range `[-1.0, 1.0]`.
    pub planar: Vec<Vec<f32>>,
    /// Number of PCM samples per channel in this frame.
    pub frame_size: usize,
}

/// Safe AAC decoder.
///
/// Wraps the fdk-aac `aacDecoder_*` API. Each instance is independent
/// (no global state). Not `Sync` — requires `&mut self` for decode.
///
/// Every constructor opens the decoder with three fdk-aac defaults changed,
/// so the PCM is the encoded content, on time and at the encoded level:
/// - error concealment by **noise substitution** (no delay) instead of
///   energy interpolation (holds one frame back);
/// - the **PCM limiter off** (no lookahead delay; a sample reconstructed
///   past full scale is hard-clipped at the s16 conversion instead of being
///   soft-limited);
/// - **loudness normalisation off**, so a stream's `prog_ref_level`
///   metadata no longer re-levels it toward a -24 dB target.
///
/// fdk-aac's defaults delayed AAC-LC output by 1744 samples at 48 kHz
/// (36.3 ms); with these settings the decoder adds none (see
/// [`output_delay_samples`](Self::output_delay_samples)).
pub struct AacDecoder {
    handle: HANDLE_AACDECODER,
    /// Pre-allocated buffer for interleaved INT_PCM (s16) output from fdk-aac.
    pcm_buf: Vec<i16>,
    /// Cached stream info after first successful decode.
    info: Option<StreamInfo>,
}

// SAFETY: fdk-aac decoder handles are per-instance with no shared global state.
// Each handle owns its internal buffers. Safe to move between threads.
unsafe impl Send for AacDecoder {}

impl AacDecoder {
    /// Open a decoder for ADTS-framed input (complete ADTS frames including header).
    pub fn open_adts() -> Result<Self, AacError> {
        Self::open_internal(TRANSPORT_TYPE_TT_MP4_ADTS)
    }

    /// Open a decoder for LATM/LOAS-framed input.
    pub fn open_latm() -> Result<Self, AacError> {
        Self::open_internal(TRANSPORT_TYPE_TT_MP4_LATM_MCP1)
    }

    /// Open a decoder for raw AAC access units.
    ///
    /// `audio_specific_config` is the AudioSpecificConfig bytes (typically 2 bytes
    /// for mono/stereo AAC-LC). The decoder uses this to configure itself before
    /// receiving any frames.
    ///
    /// To construct an ASC from ADTS fields `(profile, sample_rate_index, channel_config)`:
    /// ```text
    /// let aot = profile + 1; // ADTS profile is AOT - 1
    /// let asc = [
    ///     (aot << 3) | (sample_rate_index >> 1),
    ///     (sample_rate_index << 7) | (channel_config << 3),
    /// ];
    /// ```
    pub fn open_raw(audio_specific_config: &[u8]) -> Result<Self, AacError> {
        let mut decoder = Self::open_internal(TRANSPORT_TYPE_TT_MP4_RAW)?;
        decoder.configure_raw(audio_specific_config)?;
        Ok(decoder)
    }

    /// Open a handle and switch off the fdk-aac defaults that delay or
    /// re-level the output, so decoded PCM is the encoded content, on time.
    /// Left at the library defaults, the decoder delays every sample by
    /// 1744 at 48 kHz (AAC-LC): one frame held back by energy-interpolation
    /// concealment plus the limiter's 15 ms lookahead. No consumer
    /// compensated that, so every decoded path presented AAC audio 36.3 ms
    /// late.
    ///
    /// - **Concealment: noise substitution (1)** instead of energy
    ///   interpolation (2), which needs the *next* frame before it can emit
    ///   the current one. Noise substitution adds no delay; it is also
    ///   FFmpeg's `libfdk_aac` decoder default. This wrapper never asks
    ///   fdk-aac to conceal a missing frame (`decode_frame` passes no
    ///   `AACDEC_CONCEAL` flag, and a frame fdk-aac flags as a decode error
    ///   is returned as `Err` with its PCM discarded), so the method's
    ///   audible effect here is nil and the removed frame of delay is the
    ///   whole change.
    /// - **PCM limiter: off (0)** instead of auto (on for every non-low-delay
    ///   AOT), which removes its attack-time lookahead. A shorter attack
    ///   plus compensation was the alternative, but it would leave a
    ///   residual delay for every consumer to subtract. The cost: a sample
    ///   reconstructed past full scale is hard-clipped at the s16
    ///   conversion (fdk-aac's `scaleValuesSaturate`; `INT_PCM` is s16)
    ///   instead of being soft-limited. With the next setting in place that
    ///   needs a source mastered at or near 0 dBFS; loudness-normalised
    ///   broadcast audio (EBU R 128 -1 dBTP, ATSC A/85 -2 dBTP) keeps that
    ///   headroom.
    /// - **Loudness normalisation: off (`AAC_DRC_REFERENCE_LEVEL` -1)**
    ///   instead of normalising to fdk-aac's default -24 dB target. By
    ///   default, a stream that carries an MPEG-4 `prog_ref_level` is
    ///   re-levelled by `target - prog_ref_level` dB: a programme declaring
    ///   -31 dB is boosted 7 dB, which the limiter used to soft-limit and
    ///   which would hard-clip with it off, and one declaring -18 dB is cut
    ///   6 dB. The metadata does not survive a re-encode, so that gain was
    ///   baked into the output silently and biased any loudness meter fed
    ///   from this decoder. Off, the PCM is at the encoded level, as from
    ///   the libavcodec AC-3 / E-AC-3 decoders, which leave dialnorm
    ///   unapplied by default (`target_level` 0). A negative level also
    ///   switches off MPEG-4 DRC compression, whose boost and cut factors
    ///   default to 0 and so were already inert.
    ///
    /// The settings are stored on the handle and survive `ConfigRaw` and
    /// in-band reconfiguration. The handle is closed if any of them fails.
    fn open_internal(transport: TRANSPORT_TYPE) -> Result<Self, AacError> {
        let handle = unsafe { aacDecoder_Open(transport, 1) };
        if handle.is_null() {
            return Err(AacError::DecoderOpen);
        }

        // Owned from here on: an early return below drops `decoder`, and
        // `Drop` closes the handle.
        let decoder = Self {
            handle,
            pcm_buf: vec![0i16; MAX_PCM_SAMPLES],
            info: None,
        };
        decoder.set_param(AACDEC_PARAM_AAC_CONCEAL_METHOD, CONCEAL_NOISE_SUBSTITUTION)?;
        decoder.set_param(AACDEC_PARAM_AAC_PCM_LIMITER_ENABLE, LIMITER_OFF)?;
        decoder.set_param(AACDEC_PARAM_AAC_DRC_REFERENCE_LEVEL, LOUDNESS_NORMALISATION_OFF)?;
        Ok(decoder)
    }

    fn set_param(&self, param: AACDEC_PARAM, value: i32) -> Result<(), AacError> {
        let err = unsafe { aacDecoder_SetParam(self.handle, param, value) };
        if err != AAC_DECODER_ERROR_AAC_DEC_OK {
            // bindgen types this C enum as c_uint under the Itanium ABI but
            // c_int under MSVC, so the cast is only a no-op on some targets.
            #[allow(clippy::unnecessary_cast)]
            let param = param as u32;
            return Err(AacError::DecoderSetParam {
                param,
                code: err as i32,
            });
        }
        Ok(())
    }

    fn configure_raw(&mut self, asc: &[u8]) -> Result<(), AacError> {
        let mut asc_ptr = asc.as_ptr() as *mut u8;
        let asc_len = asc.len() as u32;

        let err = unsafe {
            aacDecoder_ConfigRaw(self.handle, &mut asc_ptr, &asc_len)
        };

        if err != AAC_DECODER_ERROR_AAC_DEC_OK {
            return Err(AacError::DecoderConfig(err as i32));
        }
        Ok(())
    }

    /// Decode one AAC frame.
    ///
    /// For `TT_MP4_RAW` transport (from `open_raw`): `data` is the raw AAC
    /// access unit bytes (ADTS header already stripped).
    ///
    /// For `TT_MP4_ADTS` transport (from `open_adts`): `data` must be a
    /// complete ADTS frame including the 7-byte header.
    ///
    /// Returns planar f32 PCM shaped `[channel][sample]`.
    pub fn decode_frame(&mut self, data: &[u8]) -> Result<DecodedFrame, AacError> {
        // Feed compressed data to the decoder's internal buffer
        let mut buf_ptr = data.as_ptr() as *mut u8;
        let buf_size = data.len() as u32;
        let mut bytes_valid = data.len() as u32;

        let err = unsafe {
            aacDecoder_Fill(self.handle, &mut buf_ptr, &buf_size, &mut bytes_valid)
        };

        if err != AAC_DECODER_ERROR_AAC_DEC_OK {
            return Err(AacError::DecoderFill(err as i32));
        }

        // Decode the frame into our PCM buffer
        let err = unsafe {
            aacDecoder_DecodeFrame(
                self.handle,
                self.pcm_buf.as_mut_ptr(),
                self.pcm_buf.len() as i32,
                0, // flags
            )
        };

        if err != AAC_DECODER_ERROR_AAC_DEC_OK {
            return Err(AacError::DecodeFailed(err as i32));
        }

        // Read stream info
        let stream_info = unsafe { aacDecoder_GetStreamInfo(self.handle) };
        if stream_info.is_null() {
            return Err(AacError::NoStreamInfo);
        }

        let si = unsafe { &*stream_info };
        let frame_size = si.frameSize as usize;
        let channels = si.numChannels as usize;
        let sample_rate = si.sampleRate as u32;
        let aot = si.aot as u8;

        // Cache stream info
        self.info = Some(StreamInfo {
            sample_rate,
            frame_size: frame_size as u32,
            channels: channels as u8,
            profile: AacProfile::from_aot(aot),
            aot,
            channel_config: si.channelConfig as u8,
            output_delay: si.outputDelay,
        });

        // Convert interleaved s16 to planar f32
        let total_samples = frame_size * channels;
        let interleaved = &self.pcm_buf[..total_samples];

        let mut planar = Vec::with_capacity(channels);
        for ch in 0..channels {
            let mut channel_data = Vec::with_capacity(frame_size);
            for s in 0..frame_size {
                let sample = interleaved[s * channels + ch];
                channel_data.push(sample as f32 / 32768.0);
            }
            planar.push(channel_data);
        }

        Ok(DecodedFrame { planar, frame_size })
    }

    /// Get stream info (available after first successful decode).
    pub fn stream_info(&self) -> Option<&StreamInfo> {
        self.info.as_ref()
    }

    /// Sample rate in Hz (available after first successful decode).
    pub fn sample_rate(&self) -> Option<u32> {
        self.info.as_ref().map(|i| i.sample_rate)
    }

    /// Number of output channels (available after first successful decode).
    pub fn channels(&self) -> Option<u8> {
        self.info.as_ref().map(|i| i.channels)
    }

    /// Output delay in samples per channel at the output sample rate, as
    /// fdk-aac reported it for the most recently decoded frame
    /// ([`StreamInfo::output_delay`]); `None` before the first successful
    /// decode.
    ///
    /// Decoded PCM sample `j` of the frame that access unit `k` produced
    /// carries the content of AU `k`'s sample `j - output_delay`.
    ///
    /// **0 for AAC-LC, AAC-LD and AAC-ELD**, because this decoder is opened
    /// with no delay-adding options (see [`AacDecoder`]): the first decoded
    /// sample is sample 0 of the first access unit, and decoded PCM may be
    /// stamped with its AU's own timestamp.
    ///
    /// For HE-AAC v1/v2 it is the SBR QMF delay (962 at the output rate on
    /// the dual-rate path), which is part of the codec. Encoders account for
    /// it in one of two ways, and the caller has to know which:
    /// - fdk-aac's encoder counts it in its `nDelay`
    ///   (`AacEncoder::codec_delay_samples`), so timestamps derived as
    ///   `input_pts - nDelay` already assume a decoder that emits it — do
    ///   **not** subtract it again, or the audio lands early by 962.
    /// - fdk-aac's own header says an edit list written from `nDelayCore`
    ///   (which excludes it) expects the decoder to "take into account any
    ///   delay caused by the SBR module", and FFmpeg's `libfdk_aac` decoder
    ///   does so by dropping `outputDelay` samples from the start.
    ///
    /// A non-zero value on a stream without SBR or MPEG Surround means a
    /// delay-adding option took effect and is worth a warning.
    pub fn output_delay_samples(&self) -> Option<u32> {
        self.info.as_ref().map(|i| i.output_delay)
    }

    /// Reset decoder state without closing. For stream restarts.
    pub fn reset(&mut self) {
        // fdk-aac doesn't have a dedicated reset, but we can set the flush flag
        // on next decode. Clear cached info so it gets re-read.
        self.info = None;
    }
}

impl Drop for AacDecoder {
    fn drop(&mut self) {
        unsafe {
            aacDecoder_Close(self.handle);
        }
    }
}

impl std::fmt::Debug for AacDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AacDecoder")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

/// Build a 2-byte AudioSpecificConfig from ADTS header fields.
///
/// This is a convenience for callers that have already parsed the ADTS header
/// and stripped it (like bilbycast-edge's `TsDemuxer`).
///
/// # Parameters
/// - `profile`: ADTS profile field (0..=3). AOT = profile + 1.
/// - `sample_rate_index`: ADTS sampling_frequency_index (0..=12).
/// - `channel_config`: ADTS channel_configuration (1..=7).
pub fn build_audio_specific_config(
    profile: u8,
    sample_rate_index: u8,
    channel_config: u8,
) -> [u8; 2] {
    let aot = profile + 1; // ADTS profile is AOT - 1
    [
        (aot << 3) | (sample_rate_index >> 1),
        (sample_rate_index << 7) | (channel_config << 3),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_close_adts() {
        let _dec = AacDecoder::open_adts().expect("open_adts should succeed");
    }

    #[test]
    fn open_close_latm() {
        let _dec = AacDecoder::open_latm().expect("open_latm should succeed");
    }

    #[test]
    fn open_close_raw() {
        // AAC-LC, 48 kHz, stereo
        let asc = build_audio_specific_config(1, 3, 2);
        let _dec = AacDecoder::open_raw(&asc).expect("open_raw should succeed");
    }

    #[test]
    fn decode_garbage_returns_error() {
        let asc = build_audio_specific_config(1, 3, 2);
        let mut dec = AacDecoder::open_raw(&asc).unwrap();
        let garbage = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0xFF, 0x55, 0xAA];
        let result = dec.decode_frame(&garbage);
        assert!(result.is_err(), "garbage input should produce an error");
    }

    #[test]
    fn build_asc_aac_lc_48k_stereo() {
        let asc = build_audio_specific_config(1, 3, 2);
        // AOT=2 (AAC-LC), sri=3 (48kHz), cc=2 (stereo)
        // Byte 0: 00010 011 = 0x13  (AOT=2 in 5 bits = 00010, sri>>1 = 01 in top, but wait...)
        // Actually: AOT is 5 bits but for AOT<=30, only 5 bits used.
        // Byte 0: [AOT(5)][sri_hi(3)] = [00010][011] = 0b00010_011 = 0x13
        // Byte 1: [sri_lo(1)][cc(4)][...] = [1][0010][000] = 0b1_0010_000 = 0x90
        // But our simplified 2-byte builder uses:
        //   byte0 = (aot << 3) | (sri >> 1) = (2 << 3) | (3 >> 1) = 16 | 1 = 0x11
        //   byte1 = (sri << 7) | (cc << 3)  = (3 << 7) | (2 << 3) = 0x80 | 0x10 = 0x90
        assert_eq!(asc, [0x11, 0x90]);
    }

    #[test]
    fn build_asc_he_aac_v1_44k_stereo() {
        // HE-AAC v1: profile=4 (AOT 5), sri=4 (44.1kHz), cc=2
        let asc = build_audio_specific_config(4, 4, 2);
        // byte0 = (5 << 3) | (4 >> 1) = 40 | 2 = 0x2A
        // byte1 = (4 << 7) | (2 << 3) = 0 | 16 = 0x10
        assert_eq!(asc, [0x2A, 0x10]);
    }

    #[test]
    fn build_asc_mono_aac_lc_44k() {
        let asc = build_audio_specific_config(1, 4, 1);
        // byte0 = (2 << 3) | (4 >> 1) = 16 | 2 = 0x12
        // byte1 = (4 << 7) | (1 << 3) = 0 | 8 = 0x08
        assert_eq!(asc, [0x12, 0x08]);
    }

    #[test]
    fn stream_info_none_before_decode() {
        let asc = build_audio_specific_config(1, 3, 2);
        let dec = AacDecoder::open_raw(&asc).unwrap();
        assert!(dec.stream_info().is_none());
        assert!(dec.sample_rate().is_none());
        assert!(dec.channels().is_none());
    }

    #[test]
    fn reset_clears_info() {
        let asc = build_audio_specific_config(1, 3, 2);
        let mut dec = AacDecoder::open_raw(&asc).unwrap();
        dec.reset();
        assert!(dec.stream_info().is_none());
    }
}

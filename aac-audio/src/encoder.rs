// Copyright (c) 2026 Softside Tech Pty Ltd. All rights reserved.
// SPDX-License-Identifier: MPL-2.0

//! Safe AAC encoder wrapping `aacEncoder_*` FFI calls.
//!
//! Supports AAC-LC, HE-AAC v1 (SBR), HE-AAC v2 (PS), AAC-LD, AAC-ELD.
//! Input is planar f32 PCM. Output is AAC bitstream (ADTS or raw).

use aac_codec::{
    AacError, AacProfile, ChannelMode, EncoderConfig, SbrSignaling, TransportType,
};
use libfdk_aac_sys::*;

/// Maximum output buffer size for one encoded frame (generous upper bound).
const MAX_OUTPUT_BYTES: usize = 8192;

/// Result of encoding one AAC frame.
#[derive(Debug)]
pub struct EncodedData {
    /// Encoded AAC bitstream bytes (ADTS frame or raw access unit).
    pub bytes: Vec<u8>,
    /// Number of input samples consumed per channel.
    pub num_samples: u32,
}

/// Safe AAC encoder.
///
/// Wraps the fdk-aac `aacEncoder_*` API. Each instance is independent
/// (no global state). Not `Sync` — requires `&mut self` for encode.
pub struct AacEncoder {
    handle: HANDLE_AACENCODER,
    /// Frame size in samples per channel (from encoder info).
    frame_size: u32,
    /// Number of configured channels.
    channels: u8,
    /// AudioSpecificConfig bytes from the encoder (for signaling).
    audio_specific_config: Vec<u8>,
    /// Pre-allocated output buffer.
    out_buf: Vec<u8>,
    /// Scratch buffer for planar f32 → interleaved s16 conversion.
    pcm_scratch: Vec<i16>,
}

// SAFETY: fdk-aac encoder handles are per-instance with no shared global state.
unsafe impl Send for AacEncoder {}

impl AacEncoder {
    /// Create and initialize an encoder with the given configuration.
    pub fn open(config: &EncoderConfig) -> Result<Self, AacError> {
        // Validate
        if config.channels == 0 || config.channels > 8 {
            return Err(AacError::UnsupportedChannelConfig(config.channels));
        }
        if config.profile == AacProfile::HeAacV2 && config.channels != 2 {
            return Err(AacError::InvalidInput(
                "HE-AAC v2 (PS) requires exactly 2 channels (stereo)".into(),
            ));
        }

        let mut handle: HANDLE_AACENCODER = std::ptr::null_mut();
        let err = unsafe { aacEncOpen(&mut handle, 0, config.channels as u32) };
        if err != AACENC_ERROR_AACENC_OK {
            return Err(AacError::EncoderOpen(err as i32));
        }

        let mut encoder = Self {
            handle,
            frame_size: 0,
            channels: config.channels,
            audio_specific_config: Vec::new(),
            out_buf: vec![0u8; MAX_OUTPUT_BYTES],
            pcm_scratch: Vec::new(),
        };

        encoder.configure(config)?;
        encoder.init()?;
        encoder.read_info()?;

        // Pre-allocate scratch buffer
        encoder.pcm_scratch = vec![0i16; encoder.frame_size as usize * config.channels as usize];

        Ok(encoder)
    }

    fn configure(&mut self, config: &EncoderConfig) -> Result<(), AacError> {
        // Audio Object Type
        self.set_param(AACENC_PARAM_AACENC_AOT, config.profile.aot() as u32)?;

        // Sample rate
        self.set_param(AACENC_PARAM_AACENC_SAMPLERATE, config.sample_rate)?;

        // Channel mode
        let ch_mode = match ChannelMode::from_channels(config.channels) {
            Some(ChannelMode::Mono) => CHANNEL_MODE_MODE_1,
            Some(ChannelMode::Stereo) => CHANNEL_MODE_MODE_2,
            Some(ChannelMode::Surround30) => CHANNEL_MODE_MODE_1_2,
            Some(ChannelMode::Surround40) => CHANNEL_MODE_MODE_1_2_1,
            Some(ChannelMode::Surround50) => CHANNEL_MODE_MODE_1_2_2,
            Some(ChannelMode::Surround51) => CHANNEL_MODE_MODE_1_2_2_1,
            Some(ChannelMode::Surround71) => CHANNEL_MODE_MODE_7_1_BACK,
            None => return Err(AacError::UnsupportedChannelConfig(config.channels)),
        };
        self.set_param(AACENC_PARAM_AACENC_CHANNELMODE, ch_mode as u32)?;

        // Channel order: WAV ordering (interleaved, L then R, etc.)
        self.set_param(AACENC_PARAM_AACENC_CHANNELORDER, 1)?;

        // Bitrate
        self.set_param(AACENC_PARAM_AACENC_BITRATE, config.bitrate)?;

        // Transport type
        let tt = match config.transport {
            TransportType::Adts => 2,  // TT_MP4_ADTS
            TransportType::Latm => 10, // TT_MP4_LATM_MCP1
            TransportType::Raw => 0,   // TT_MP4_RAW
        };
        self.set_param(AACENC_PARAM_AACENC_TRANSMUX, tt)?;

        // Afterburner (quality enhancer)
        self.set_param(
            AACENC_PARAM_AACENC_AFTERBURNER,
            if config.afterburner { 1 } else { 0 },
        )?;

        // SBR signaling (for HE-AAC)
        if config.profile == AacProfile::HeAacV1 || config.profile == AacProfile::HeAacV2 {
            let sig = match config.sbr_signaling {
                SbrSignaling::Implicit => 0,
                SbrSignaling::ExplicitBackwardCompatible => 1,
                SbrSignaling::ExplicitHierarchical => 2,
            };
            self.set_param(AACENC_PARAM_AACENC_SIGNALING_MODE, sig)?;
        }

        Ok(())
    }

    fn set_param(&self, param: AACENC_PARAM, value: u32) -> Result<(), AacError> {
        let err = unsafe { aacEncoder_SetParam(self.handle, param, value) };
        if err != AACENC_ERROR_AACENC_OK {
            return Err(AacError::EncoderSetParam {
                param: param as u32,
                code: err as i32,
            });
        }
        Ok(())
    }

    fn init(&self) -> Result<(), AacError> {
        // Call aacEncEncode with all-null args to trigger initialization
        let err = unsafe {
            aacEncEncode(
                self.handle,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        };
        if err != AACENC_ERROR_AACENC_OK {
            return Err(AacError::EncoderInit(err as i32));
        }
        Ok(())
    }

    fn read_info(&mut self) -> Result<(), AacError> {
        let mut info: AACENC_InfoStruct = unsafe { std::mem::zeroed() };
        let err = unsafe { aacEncInfo(self.handle, &mut info) };
        if err != AACENC_ERROR_AACENC_OK {
            return Err(AacError::EncoderInfo(err as i32));
        }

        self.frame_size = info.frameLength as u32;

        // Extract AudioSpecificConfig
        let asc_len = info.confSize as usize;
        if asc_len > 0 && asc_len <= info.confBuf.len() {
            self.audio_specific_config = info.confBuf[..asc_len].iter().map(|&b| b as u8).collect();
        }

        Ok(())
    }

    /// Encode one frame of planar f32 PCM.
    ///
    /// `planar` must have exactly `channels()` inner vecs, each with exactly
    /// `frame_size()` samples. Values should be in `[-1.0, 1.0]`.
    ///
    /// Returns the encoded AAC bitstream bytes.
    pub fn encode_frame(&mut self, planar: &[Vec<f32>]) -> Result<EncodedData, AacError> {
        let channels = self.channels as usize;
        let frame_size = self.frame_size as usize;

        if planar.len() != channels {
            return Err(AacError::InvalidInput(format!(
                "expected {} channels, got {}",
                channels,
                planar.len()
            )));
        }

        // Convert planar f32 to interleaved s16
        for s in 0..frame_size {
            for ch in 0..channels {
                let sample = if s < planar[ch].len() {
                    planar[ch][s]
                } else {
                    0.0
                };
                self.pcm_scratch[s * channels + ch] =
                    (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            }
        }

        self.encode_interleaved_s16()
    }

    /// Encode from interleaved s16 PCM directly.
    ///
    /// `interleaved` must have exactly `frame_size() * channels()` samples.
    /// This avoids the f32→s16 conversion when the caller already has s16 data.
    pub fn encode_frame_s16(&mut self, interleaved: &[i16]) -> Result<EncodedData, AacError> {
        let expected = self.frame_size as usize * self.channels as usize;
        if interleaved.len() != expected {
            return Err(AacError::InvalidInput(format!(
                "expected {} interleaved s16 samples, got {}",
                expected,
                interleaved.len()
            )));
        }
        self.pcm_scratch[..expected].copy_from_slice(interleaved);
        self.encode_interleaved_s16()
    }

    fn encode_interleaved_s16(&mut self) -> Result<EncodedData, AacError> {
        let channels = self.channels as usize;
        let frame_size = self.frame_size as usize;
        let num_input_samples = (frame_size * channels) as i32;
        let input_bytes = num_input_samples as i32 * 2; // s16 = 2 bytes per sample

        // Set up input buffer descriptor
        let mut in_buf_ptr = self.pcm_scratch.as_mut_ptr() as *mut std::ffi::c_void;
        let mut in_buf_id: i32 = 0; // IN_AUDIO_DATA
        let mut in_buf_size: i32 = input_bytes;
        let mut in_buf_el_size: i32 = 2; // sizeof(INT_PCM) = sizeof(i16) = 2

        let in_buf_desc = AACENC_BufDesc {
            numBufs: 1,
            bufs: &mut in_buf_ptr,
            bufferIdentifiers: &mut in_buf_id,
            bufSizes: &mut in_buf_size,
            bufElSizes: &mut in_buf_el_size,
        };

        // Set up output buffer descriptor
        let mut out_buf_ptr = self.out_buf.as_mut_ptr() as *mut std::ffi::c_void;
        let mut out_buf_id: i32 = 3; // OUT_BITSTREAM_DATA
        let mut out_buf_size: i32 = self.out_buf.len() as i32;
        let mut out_buf_el_size: i32 = 1; // bytes

        let out_buf_desc = AACENC_BufDesc {
            numBufs: 1,
            bufs: &mut out_buf_ptr,
            bufferIdentifiers: &mut out_buf_id,
            bufSizes: &mut out_buf_size,
            bufElSizes: &mut out_buf_el_size,
        };

        let in_args = AACENC_InArgs {
            numInSamples: num_input_samples,
            numAncBytes: 0,
        };

        let mut out_args: AACENC_OutArgs = unsafe { std::mem::zeroed() };

        let err = unsafe {
            aacEncEncode(
                self.handle,
                &in_buf_desc,
                &out_buf_desc,
                &in_args,
                &mut out_args,
            )
        };

        if err != AACENC_ERROR_AACENC_OK {
            return Err(AacError::EncodeFailed(err as i32));
        }

        let out_bytes = out_args.numOutBytes as usize;
        Ok(EncodedData {
            bytes: self.out_buf[..out_bytes].to_vec(),
            num_samples: self.frame_size,
        })
    }

    /// Required frame size in samples per channel.
    /// - 1024 for AAC-LC
    /// - 2048 for HE-AAC v1/v2 (SBR doubles)
    /// - 480/512 for AAC-LD/ELD
    pub fn frame_size(&self) -> u32 {
        self.frame_size
    }

    /// Number of configured channels.
    pub fn channels(&self) -> u8 {
        self.channels
    }

    /// Get the AudioSpecificConfig bytes (for signaling to decoders, e.g. in
    /// FLV AudioSpecificConfig sequence headers or SDP fmtp).
    pub fn audio_specific_config(&self) -> &[u8] {
        &self.audio_specific_config
    }

    /// Flush the encoder (end of stream). Returns any remaining encoded data,
    /// or `None` if the encoder has no buffered data.
    pub fn flush(&mut self) -> Result<Option<EncodedData>, AacError> {
        // Feed zero-length input to signal EOF
        let mut in_buf_ptr = std::ptr::null_mut();
        let mut in_buf_id: i32 = 0; // IN_AUDIO_DATA
        let mut in_buf_size: i32 = 0;
        let mut in_buf_el_size: i32 = 2;

        let in_buf_desc = AACENC_BufDesc {
            numBufs: 1,
            bufs: &mut in_buf_ptr,
            bufferIdentifiers: &mut in_buf_id,
            bufSizes: &mut in_buf_size,
            bufElSizes: &mut in_buf_el_size,
        };

        let mut out_buf_ptr = self.out_buf.as_mut_ptr() as *mut std::ffi::c_void;
        let mut out_buf_id: i32 = 3; // OUT_BITSTREAM_DATA
        let mut out_buf_size: i32 = self.out_buf.len() as i32;
        let mut out_buf_el_size: i32 = 1;

        let out_buf_desc = AACENC_BufDesc {
            numBufs: 1,
            bufs: &mut out_buf_ptr,
            bufferIdentifiers: &mut out_buf_id,
            bufSizes: &mut out_buf_size,
            bufElSizes: &mut out_buf_el_size,
        };

        let in_args = AACENC_InArgs {
            numInSamples: -1, // EOF signal
            numAncBytes: 0,
        };

        let mut out_args: AACENC_OutArgs = unsafe { std::mem::zeroed() };

        let err = unsafe {
            aacEncEncode(
                self.handle,
                &in_buf_desc,
                &out_buf_desc,
                &in_args,
                &mut out_args,
            )
        };

        if err != AACENC_ERROR_AACENC_OK {
            return Err(AacError::EncodeFailed(err as i32));
        }

        let out_bytes = out_args.numOutBytes as usize;
        if out_bytes == 0 {
            Ok(None)
        } else {
            Ok(Some(EncodedData {
                bytes: self.out_buf[..out_bytes].to_vec(),
                num_samples: 0,
            }))
        }
    }
}

impl Drop for AacEncoder {
    fn drop(&mut self) {
        unsafe {
            aacEncClose(&mut self.handle);
        }
    }
}

impl std::fmt::Debug for AacEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AacEncoder")
            .field("frame_size", &self.frame_size)
            .field("channels", &self.channels)
            .field("asc_len", &self.audio_specific_config.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_aac_lc_stereo() {
        let config = EncoderConfig::aac_lc(48000, 2, 128_000);
        let enc = AacEncoder::open(&config).expect("AAC-LC stereo open should succeed");
        assert_eq!(enc.frame_size(), 1024);
        assert_eq!(enc.channels(), 2);
        assert!(!enc.audio_specific_config().is_empty());
    }

    #[test]
    fn open_aac_lc_mono() {
        let config = EncoderConfig::aac_lc(48000, 1, 64_000);
        let enc = AacEncoder::open(&config).expect("AAC-LC mono open should succeed");
        assert_eq!(enc.frame_size(), 1024);
        assert_eq!(enc.channels(), 1);
    }

    #[test]
    fn open_he_aac_v1_stereo() {
        let config = EncoderConfig::he_aac_v1(44100, 2, 64_000);
        let enc = AacEncoder::open(&config).expect("HE-AAC v1 open should succeed");
        // HE-AAC v1 typically has frame_size of 2048 (SBR doubles)
        assert!(enc.frame_size() >= 1024);
        assert_eq!(enc.channels(), 2);
    }

    #[test]
    fn open_he_aac_v2_stereo() {
        let config = EncoderConfig::he_aac_v2(44100, 32_000);
        let enc = AacEncoder::open(&config).expect("HE-AAC v2 open should succeed");
        assert_eq!(enc.channels(), 2);
    }

    #[test]
    fn he_aac_v2_rejects_mono() {
        let config = EncoderConfig {
            profile: AacProfile::HeAacV2,
            channels: 1,
            sample_rate: 44100,
            bitrate: 32_000,
            afterburner: true,
            sbr_signaling: SbrSignaling::default(),
            transport: TransportType::Adts,
        };
        let err = AacEncoder::open(&config).unwrap_err();
        assert!(
            matches!(err, AacError::InvalidInput(_)),
            "HE-AAC v2 mono should be rejected: {err:?}"
        );
    }

    #[test]
    fn encode_silence_aac_lc() {
        let config = EncoderConfig::aac_lc(48000, 2, 128_000);
        let mut enc = AacEncoder::open(&config).unwrap();

        // One frame of silence
        let silence = vec![vec![0.0f32; 1024]; 2];
        let result = enc.encode_frame(&silence).expect("encoding silence should succeed");
        assert!(!result.bytes.is_empty(), "encoded output should not be empty");
        assert_eq!(result.num_samples, 1024);

        // For ADTS, check sync word
        if result.bytes.len() >= 2 {
            assert_eq!(result.bytes[0], 0xFF);
            assert_eq!(result.bytes[1] & 0xF0, 0xF0, "ADTS sync word mismatch");
        }
    }

    #[test]
    fn encode_wrong_channel_count_rejected() {
        let config = EncoderConfig::aac_lc(48000, 2, 128_000);
        let mut enc = AacEncoder::open(&config).unwrap();

        // Pass 3 channels when encoder expects 2
        let wrong = vec![vec![0.0f32; 1024]; 3];
        let err = enc.encode_frame(&wrong).unwrap_err();
        assert!(matches!(err, AacError::InvalidInput(_)));
    }

    #[test]
    fn round_trip_encode_decode() {
        // Encode silence, then decode it
        let enc_config = EncoderConfig::aac_lc(48000, 2, 128_000);
        let mut enc = AacEncoder::open(&enc_config).unwrap();

        let silence = vec![vec![0.0f32; 1024]; 2];

        // Encode several frames (decoder may need a few frames to produce output)
        let mut encoded_frames = Vec::new();
        for _ in 0..5 {
            let result = enc.encode_frame(&silence).unwrap();
            encoded_frames.push(result.bytes);
        }

        // Decode with ADTS transport (encoder outputs ADTS)
        let mut dec = crate::AacDecoder::open_adts().unwrap();
        let mut decoded_count = 0;

        for frame in &encoded_frames {
            match dec.decode_frame(frame) {
                Ok(decoded) => {
                    assert_eq!(decoded.planar.len(), 2, "expected 2 channels");
                    assert_eq!(decoded.frame_size, 1024, "expected 1024 samples per frame");
                    decoded_count += 1;
                }
                Err(_) => {
                    // First frame may fail (decoder priming)
                }
            }
        }

        assert!(decoded_count > 0, "should have decoded at least one frame");

        // Verify stream info
        let info = dec.stream_info().expect("stream info should be available");
        assert_eq!(info.sample_rate, 48000);
        assert_eq!(info.channels, 2);
    }
}

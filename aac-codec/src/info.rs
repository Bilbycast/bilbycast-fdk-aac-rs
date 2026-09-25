// Copyright (c) 2026 Softside Tech Pty Ltd. All rights reserved.
// SPDX-License-Identifier: MPL-2.0

//! Stream information extracted from decoded AAC frames.

use crate::AacProfile;

/// Stream information available after the first successful decode.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    /// Output sample rate in Hz (after SBR upsampling if applicable).
    pub sample_rate: u32,
    /// Number of PCM samples per channel per frame.
    /// - 1024 for AAC-LC
    /// - 2048 for HE-AAC v1/v2 (SBR doubles the core frame)
    /// - 480/512 for AAC-LD/ELD
    pub frame_size: u32,
    /// Number of output channels.
    pub channels: u8,
    /// Detected AAC profile (if mappable to a known variant).
    pub profile: Option<AacProfile>,
    /// Raw Audio Object Type from the bitstream.
    pub aot: u8,
    /// Raw channel configuration from the bitstream.
    pub channel_config: u8,
    /// Samples per channel, at `sample_rate`, by which the decoder reports
    /// delaying its output (fdk-aac `CStreamInfo.outputDelay`, re-read on
    /// every decoded frame).
    ///
    /// `aac-audio` opens every decoder with noise-substitution concealment
    /// and the PCM limiter off, and neither adds delay, so this is **0 for
    /// AAC-LC, AAC-LD and AAC-ELD**. fdk-aac's own defaults would report
    /// 1744 for AAC-LC at 48 kHz: one frame (1024) held back by energy-
    /// interpolation concealment plus the limiter's 15 ms attack (720).
    ///
    /// For SBR streams (HE-AAC v1/v2) what remains is the SBR QMF-bank delay
    /// (962 samples at the output rate on the usual dual-rate path, 481 when
    /// SBR runs downsampled), and MPEG Surround adds its own figure. That
    /// part belongs to the codec rather than to fdk-aac's settings and cannot
    /// be switched off. Whether a caller should subtract it depends on how
    /// the source encoder stamped its timestamps — see
    /// `aac_audio::AacDecoder::output_delay_samples`.
    pub output_delay: u32,
}

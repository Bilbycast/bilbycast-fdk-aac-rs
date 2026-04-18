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
}

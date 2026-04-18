// Copyright (c) 2026 Softside Tech Pty Ltd. All rights reserved.
// SPDX-License-Identifier: MPL-2.0

//! AAC codec configuration types.

/// AAC Audio Object Type (ISO 14496-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AacProfile {
    /// AAC-LC (Low Complexity) — AOT 2. Most common profile.
    AacLc,
    /// HE-AAC v1 (High Efficiency with SBR) — AOT 5.
    HeAacV1,
    /// HE-AAC v2 (High Efficiency with SBR + PS) — AOT 29. Stereo only.
    HeAacV2,
    /// AAC-LD (Low Delay) — AOT 23. For low-latency contribution.
    AacLd,
    /// AAC-ELD (Enhanced Low Delay) — AOT 39.
    AacEld,
}

impl AacProfile {
    /// Return the ISO 14496-3 Audio Object Type number.
    pub fn aot(self) -> u8 {
        match self {
            AacProfile::AacLc => 2,
            AacProfile::HeAacV1 => 5,
            AacProfile::HeAacV2 => 29,
            AacProfile::AacLd => 23,
            AacProfile::AacEld => 39,
        }
    }

    /// Try to map an AOT number back to a profile.
    pub fn from_aot(aot: u8) -> Option<Self> {
        match aot {
            2 => Some(AacProfile::AacLc),
            5 => Some(AacProfile::HeAacV1),
            29 => Some(AacProfile::HeAacV2),
            23 => Some(AacProfile::AacLd),
            39 => Some(AacProfile::AacEld),
            _ => None,
        }
    }
}

impl std::fmt::Display for AacProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AacProfile::AacLc => write!(f, "AAC-LC"),
            AacProfile::HeAacV1 => write!(f, "HE-AAC v1"),
            AacProfile::HeAacV2 => write!(f, "HE-AAC v2"),
            AacProfile::AacLd => write!(f, "AAC-LD"),
            AacProfile::AacEld => write!(f, "AAC-ELD"),
        }
    }
}

/// Transport framing for decoder input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    /// ADTS framing (most MPEG-TS streams).
    Adts,
    /// LATM/LOAS framing (some DVB/MPEG-TS).
    Latm,
    /// Raw access units — caller handles framing and provides AudioSpecificConfig.
    Raw,
}

/// Channel configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMode {
    /// Mono (1 channel).
    Mono,
    /// Stereo (2 channels).
    Stereo,
    /// 3.0 (center + left + right).
    Surround30,
    /// 4.0 (center + left + right + rear center).
    Surround40,
    /// 5.0 (center + left + right + left surround + right surround).
    Surround50,
    /// 5.1 (5.0 + LFE).
    Surround51,
    /// 7.1 (5.1 + left back + right back).
    Surround71,
}

impl ChannelMode {
    /// Number of audio channels.
    pub fn channels(self) -> u8 {
        match self {
            ChannelMode::Mono => 1,
            ChannelMode::Stereo => 2,
            ChannelMode::Surround30 => 3,
            ChannelMode::Surround40 => 4,
            ChannelMode::Surround50 => 5,
            ChannelMode::Surround51 => 6,
            ChannelMode::Surround71 => 8,
        }
    }

    /// Map a channel count to the default channel mode.
    pub fn from_channels(ch: u8) -> Option<Self> {
        match ch {
            1 => Some(ChannelMode::Mono),
            2 => Some(ChannelMode::Stereo),
            3 => Some(ChannelMode::Surround30),
            4 => Some(ChannelMode::Surround40),
            5 => Some(ChannelMode::Surround50),
            6 => Some(ChannelMode::Surround51),
            8 => Some(ChannelMode::Surround71),
            _ => None,
        }
    }

    /// Map an ADTS/AudioSpecificConfig `channel_configuration` field (0..7)
    /// to a channel mode.
    pub fn from_channel_config(cc: u8) -> Option<Self> {
        match cc {
            1 => Some(ChannelMode::Mono),
            2 => Some(ChannelMode::Stereo),
            3 => Some(ChannelMode::Surround30),
            4 => Some(ChannelMode::Surround40),
            5 => Some(ChannelMode::Surround50),
            6 => Some(ChannelMode::Surround51),
            7 => Some(ChannelMode::Surround71),
            _ => None,
        }
    }
}

/// SBR signaling mode for HE-AAC encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbrSignaling {
    /// Implicit SBR signaling (backward compatible with AAC-LC decoders).
    Implicit,
    /// Explicit backward compatible signaling.
    ExplicitBackwardCompatible,
    /// Explicit hierarchical signaling (for MPEG-DASH).
    ExplicitHierarchical,
}

impl Default for SbrSignaling {
    fn default() -> Self {
        SbrSignaling::Implicit
    }
}

/// Encoder configuration.
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// AAC profile to encode.
    pub profile: AacProfile,
    /// Input sample rate in Hz.
    pub sample_rate: u32,
    /// Number of input channels (1-8).
    pub channels: u8,
    /// Target bitrate in bits per second.
    pub bitrate: u32,
    /// Enable afterburner (high-quality mode). Increases CPU ~10%, improves
    /// quality at low bitrates. Default: true.
    pub afterburner: bool,
    /// SBR signaling mode (only relevant for HE-AAC v1/v2).
    pub sbr_signaling: SbrSignaling,
    /// Output transport type (ADTS for streaming, Raw for container muxing).
    pub transport: TransportType,
}

impl EncoderConfig {
    /// Create a config for AAC-LC encoding with sensible defaults.
    pub fn aac_lc(sample_rate: u32, channels: u8, bitrate: u32) -> Self {
        Self {
            profile: AacProfile::AacLc,
            sample_rate,
            channels,
            bitrate,
            afterburner: true,
            sbr_signaling: SbrSignaling::default(),
            transport: TransportType::Adts,
        }
    }

    /// Create a config for HE-AAC v1 encoding with sensible defaults.
    pub fn he_aac_v1(sample_rate: u32, channels: u8, bitrate: u32) -> Self {
        Self {
            profile: AacProfile::HeAacV1,
            sample_rate,
            channels,
            bitrate,
            afterburner: true,
            sbr_signaling: SbrSignaling::default(),
            transport: TransportType::Adts,
        }
    }

    /// Create a config for HE-AAC v2 encoding. Requires stereo (2 channels).
    pub fn he_aac_v2(sample_rate: u32, bitrate: u32) -> Self {
        Self {
            profile: AacProfile::HeAacV2,
            sample_rate,
            channels: 2,
            bitrate,
            afterburner: true,
            sbr_signaling: SbrSignaling::default(),
            transport: TransportType::Adts,
        }
    }
}

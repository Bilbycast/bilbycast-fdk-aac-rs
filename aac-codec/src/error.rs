// Copyright (c) 2026 Softside Tech Pty Ltd. All rights reserved.
// SPDX-License-Identifier: MPL-2.0

//! Error types for AAC decode and encode operations.

use thiserror::Error;

/// Errors produced by the AAC decoder or encoder.
#[derive(Debug, Error)]
pub enum AacError {
    // ── Decoder errors ──────────────────────────────────────────────────

    /// `aacDecoder_Open` returned a null handle.
    #[error("AAC decoder open failed (transport type not supported or out of memory)")]
    DecoderOpen,

    /// `aacDecoder_ConfigRaw` failed.
    #[error("AAC decoder config failed: fdk-aac error code 0x{0:04X}")]
    DecoderConfig(i32),

    /// `aacDecoder_Fill` failed.
    #[error("AAC decoder fill failed: fdk-aac error code 0x{0:04X}")]
    DecoderFill(i32),

    /// `aacDecoder_DecodeFrame` failed.
    #[error("AAC decode failed: fdk-aac error code 0x{0:04X}")]
    DecodeFailed(i32),

    /// Stream info not yet available (no successful decode).
    #[error("AAC stream info not available (no frame decoded yet)")]
    NoStreamInfo,

    // ── Encoder errors ──────────────────────────────────────────────────

    /// `aacEncOpen` failed.
    #[error("AAC encoder open failed: fdk-aac error code 0x{0:04X}")]
    EncoderOpen(i32),

    /// `aacEncoder_SetParam` failed.
    #[error("AAC encoder set param failed (param 0x{param:04X}): fdk-aac error code 0x{code:04X}")]
    EncoderSetParam { param: u32, code: i32 },

    /// `aacEncEncode` init call failed.
    #[error("AAC encoder init failed: fdk-aac error code 0x{0:04X}")]
    EncoderInit(i32),

    /// `aacEncEncode` failed during encoding.
    #[error("AAC encode failed: fdk-aac error code 0x{0:04X}")]
    EncodeFailed(i32),

    /// `aacEncInfo` failed.
    #[error("AAC encoder info failed: fdk-aac error code 0x{0:04X}")]
    EncoderInfo(i32),

    // ── Shared errors ───────────────────────────────────────────────────

    /// Invalid input parameters.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Unsupported AAC profile for the requested operation.
    #[error("unsupported AAC profile: AOT {0}")]
    UnsupportedProfile(u8),

    /// Unsupported channel configuration.
    #[error("unsupported channel configuration: {0} channels")]
    UnsupportedChannelConfig(u8),
}

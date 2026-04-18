// Copyright (c) 2026 Softside Tech Pty Ltd. All rights reserved.
// SPDX-License-Identifier: MPL-2.0

//! Safe Rust wrapper around Fraunhofer FDK AAC for decode and encode.
//!
//! This crate provides:
//! - [`AacDecoder`] — decodes AAC bitstreams (ADTS, LATM, raw) to planar f32 PCM
//! - [`AacEncoder`] — encodes planar f32 PCM to AAC bitstreams
//!
//! Supports AAC-LC, HE-AAC v1 (SBR), HE-AAC v2 (PS), AAC-LD, AAC-ELD,
//! and multichannel up to 7.1.
//!
//! # Example
//!
//! ```no_run
//! use aac_audio::AacDecoder;
//!
//! // Open a decoder for raw AAC access units
//! let asc = [0x11, 0x90]; // AAC-LC, 48 kHz, stereo
//! let mut decoder = AacDecoder::open_raw(&asc).unwrap();
//!
//! // Decode a frame (raw AAC AU bytes, ADTS header already stripped)
//! // let frame = decoder.decode_frame(&aac_bytes).unwrap();
//! // frame.planar is Vec<Vec<f32>> shaped [channel][sample]
//! ```

pub mod decoder;
pub mod encoder;

pub use aac_codec::{
    AacError, AacProfile, ChannelMode, EncoderConfig, SbrSignaling, StreamInfo, TransportType,
};
pub use decoder::{AacDecoder, DecodedFrame};
pub use encoder::{AacEncoder, EncodedData};

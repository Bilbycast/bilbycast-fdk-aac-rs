// Copyright (c) 2026 Softside Tech Pty Ltd. All rights reserved.
// SPDX-License-Identifier: MPL-2.0

//! Pure Rust types for AAC codec configuration, errors, and stream info.
//!
//! This crate has zero C dependencies. It provides shared types used by both
//! the `aac-audio` safe wrapper and `bilbycast-edge`.

pub mod config;
pub mod error;
pub mod info;

pub use config::*;
pub use error::*;
pub use info::*;

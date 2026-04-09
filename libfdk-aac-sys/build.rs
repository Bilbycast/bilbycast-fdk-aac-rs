// Copyright (c) 2026 Reza Rahimi / Softside Tech Pty Ltd. All rights reserved.
// SPDX-License-Identifier: MPL-2.0

//! Build script for libfdk-aac-sys.
//!
//! Default: compile vendored fdk-aac v2.0.3 from `vendor/fdk-aac/` via CMake.
//! Override: set `LIBFDK_AAC_DIR` env var to point to a pre-built install.
//! Override: enable `system-libfdk-aac` feature to use pkg-config.

use std::env;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Determine include path and link instructions
    let include_path = if let Ok(fdk_dir) = env::var("LIBFDK_AAC_DIR") {
        // User-specified libfdk-aac install
        let fdk_path = PathBuf::from(&fdk_dir);
        println!(
            "cargo:rustc-link-search=native={}",
            fdk_path.join("lib").display()
        );
        println!("cargo:rustc-link-lib=static=fdk-aac");
        link_cpp_stdlib();
        fdk_path.join("include")
    } else if cfg!(feature = "system-libfdk-aac") {
        // System libfdk-aac via pkg-config
        let lib = pkg_config::Config::new()
            .atleast_version("2.0.0")
            .probe("fdk-aac")
            .expect(
                "pkg-config: fdk-aac >= 2.0.0 not found. \
                 Install libfdk-aac-dev or set LIBFDK_AAC_DIR",
            );
        PathBuf::from(
            lib.include_paths
                .first()
                .expect("no include path from pkg-config"),
        )
    } else {
        // Vendored build via CMake (default)
        build_vendored(&out_dir)
    };

    // Generate Rust bindings via bindgen
    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{}", include_path.display()))
        // Decoder API
        .allowlist_function("aacDecoder_.*")
        .allowlist_type("AAC_DECODER_ERROR")
        .allowlist_type("CStreamInfo")
        // Encoder API
        .allowlist_function("aacEncoder_.*")
        .allowlist_function("aacEncOpen")
        .allowlist_function("aacEncClose")
        .allowlist_function("aacEncEncode")
        .allowlist_function("aacEncInfo")
        .allowlist_type("AACENC_ERROR")
        .allowlist_type("AACENC_InfoStruct")
        .allowlist_type("AACENC_BufDesc")
        .allowlist_type("AACENC_InArgs")
        .allowlist_type("AACENC_OutArgs")
        .allowlist_type("AACENC_PARAM")
        // Shared enums/types
        .allowlist_type("TRANSPORT_TYPE")
        .allowlist_type("AUDIO_OBJECT_TYPE")
        .allowlist_type("CHANNEL_MODE")
        // Constants
        .allowlist_var("TT_.*")
        .allowlist_var("AAC_.*")
        .allowlist_var("AACENC_.*")
        .allowlist_var("AOT_.*")
        .allowlist_var("IN_.*")
        .allowlist_var("OUT_.*")
        .derive_debug(true)
        .derive_copy(true)
        .derive_default(true)
        .generate()
        .expect("bindgen failed to generate fdk-aac bindings");

    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("failed to write bindings.rs");
}

/// Build fdk-aac from vendored source using CMake.
fn build_vendored(out_dir: &PathBuf) -> PathBuf {
    let fdk_source = PathBuf::from("vendor/fdk-aac");
    if !fdk_source.exists() {
        panic!(
            "Vendored fdk-aac source not found at {}. \
             Clone it with: git submodule update --init, \
             or set LIBFDK_AAC_DIR to a pre-built install, \
             or enable the system-libfdk-aac feature.",
            fdk_source.display()
        );
    }

    let dst = cmake::Config::new(&fdk_source)
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("BUILD_PROGRAMS", "OFF")
        .define("FDK_AAC_INSTALL_CMAKE_CONFIG_MODULE", "OFF")
        .define("FDK_AAC_INSTALL_PKGCONFIG_MODULE", "OFF")
        .define("CMAKE_INSTALL_PREFIX", out_dir.to_str().unwrap())
        .build();

    let lib_dir = dst.join("lib");
    // Some systems use lib64
    let lib_dir = if lib_dir.exists() {
        lib_dir
    } else {
        dst.join("lib64")
    };

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=fdk-aac");
    link_cpp_stdlib();

    dst.join("include")
}

/// Link C++ standard library required by fdk-aac.
fn link_cpp_stdlib() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "linux" => {
            println!("cargo:rustc-link-lib=stdc++");
        }
        "macos" => {
            println!("cargo:rustc-link-lib=c++");
        }
        _ => {}
    }
}

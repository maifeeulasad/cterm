//! # cterm-proto
//!
//! The gRPC/protobuf contract for cterm (compiled from `proto/cterm.proto` by
//! [`tonic-build`]) plus conversions from `cterm-core` domain types to their
//! wire form. The daemon serves [`v1::terminal_server::Terminal`]; agents use
//! [`v1::terminal_client::TerminalClient`].

#![allow(missing_docs)] // generated protobuf code is not documented

/// Generated types for the `cterm.v1` package.
pub mod v1 {
    tonic::include_proto!("cterm.v1");
}

pub mod convert;

pub use v1::terminal_client::TerminalClient;
pub use v1::terminal_server::{Terminal, TerminalServer};

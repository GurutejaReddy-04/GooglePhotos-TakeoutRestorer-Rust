//! Google Photos Takeout Restorer - Core Crate
//! High-performance Rust pipeline for restoring EXIF metadata to Google Photos Takeout archives.
//!
//! Author: Guruteja Reddy Nallachi (<https://github.com/GurutejaReddy-04>)
//! Open Source Software released under the MIT License.

pub mod auto_heal;
pub mod config;
pub mod error;
pub mod events;
pub mod exiftool;
pub mod logger;
pub mod matcher;
pub mod parser;
pub mod processor;
pub mod scanner;
pub mod state_db;
pub mod timezone;
pub mod validation;

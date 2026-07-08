#![allow(dead_code)]
#[path = "../src/config.rs"]
mod config;
#[path = "../src/metadata.rs"]
mod metadata;
#[path = "../src/sync.rs"]
mod sync;

use config::{LosslessFormat, Mode};
use sync::resolve_output_policy;

#[test]
fn compat_mode_always_targets_mp3() {
    let policy = resolve_output_policy(Mode::Compat, None, "flac");
    assert_eq!(policy.output_extension, "mp3");
}

#[test]
fn lossless_mode_uses_requested_format() {
    let wav_policy = resolve_output_policy(Mode::Lossless, Some(LosslessFormat::Wav), "ncm");
    assert_eq!(wav_policy.output_extension, "wav");

    let flac_policy = resolve_output_policy(Mode::Lossless, Some(LosslessFormat::Flac), "ncm");
    assert_eq!(flac_policy.output_extension, "flac");

    let aiff_policy = resolve_output_policy(Mode::Lossless, Some(LosslessFormat::Aiff), "ncm");
    assert_eq!(aiff_policy.output_extension, "aiff");
}

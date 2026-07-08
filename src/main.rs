mod config;
mod sync;
mod metadata;
pub mod task;
use crate::config::{Cmd, Config};
use crate::sync::{compare_music_dicts, get_music_dict, sync_music_library_with_policy};
use clap::Parser;
use std::fs;
use std::io::{Error, ErrorKind};
use std::path::Path;
use text_to_ascii_art::to_art;

fn main() -> Result<(), Error> {
    match to_art("W4DJ".to_string(), "standard", 0, 2, 0) {
        Ok(string) => println!("{}", string),
        Err(err) => println!("Error: {}", err),
    }

    let cmd = Cmd::parse();
    let config_file_path = cmd.config.expect("Clap should provide default value");

    let config_content = fs::read_to_string(&config_file_path).map_err(|e| {
        Error::new(
            e.kind(),
            format!("Failed to read config '{}': {}", config_file_path, e),
        )
    })?;

    let config: Config = toml::from_str(&config_content).map_err(|e| {
        Error::new(
            ErrorKind::InvalidData,
            format!("Failed to parse TOML from '{}': {}", config_file_path, e),
        )
    })?;

    let Config {
        source,
        destination,
        mode,
        lossless_format,
    } = config;

    println!(
        "Config loaded: Source='{}', Destination='{}', Mode={:?}, LosslessFormat={:?}",
        source, destination, mode, lossless_format
    );

    let wf = &source;
    let sf = &destination;

    if !Path::new(wf).exists() {
        eprintln!("Source folder does not exist: {}", wf);
        return Err(Error::new(
            ErrorKind::NotFound,
            format!("Source folder not found: {}", wf),
        ));
    }

    if !Path::new(sf).exists() {
        println!("Destination folder '{}' does not exist, creating...", sf);
        fs::create_dir_all(sf)?;
    }

    println!("Scanning source folder: {}", wf);
    let wf_dict = get_music_dict(wf);
    println!("Found {} music files in source.", wf_dict.len());

    println!("Scanning destination folder: {}", sf);
    let sf_dict = get_music_dict(sf);
    println!("Found {} music files in destination.", sf_dict.len());

    let new_songs = compare_music_dicts(&wf_dict, &sf_dict, &mode);
    println!("Found {} new songs to sync.", new_songs.len());

    if !new_songs.is_empty() {
        let snapshot = sync_music_library_with_policy(&new_songs, sf, &mode, lossless_format)?;
        println!(
            "Sync status: {}/{} files processed, {} remaining.",
            snapshot.completed, snapshot.total, snapshot.remaining
        );
    }

    println!("Sync completed successfully.");
    Ok(())
}

use std::{fs, path::Path, process::Command};

use anyhow::{Context, Ok, Result};
use bwl_meta::constants::{CONFIG_TOML, FRAMES_DIR, OCR_DIR};
use bwl_meta::models::Config;
use bwl_meta::parse_segments;

fn main() -> Result<()> {
    let conf: Config = toml::from_str(CONFIG_TOML)?;
    fs::create_dir_all(FRAMES_DIR)?;
    fs::create_dir_all(OCR_DIR)?;
    let video_name = format!("{}.mp4", conf.yt_url.split_once('?').map(|s| s.1).unwrap());

    // Download yt video
    if !Path::new(&video_name).exists() {
        let status = Command::new("yt-dlp")
            .args([
                "-f",
                "bestvideo[height<=1080]",
                "--no-playlist",
                "-o",
                &video_name,
                "--merge-output-format",
                "mp4",
                &conf.yt_url,
            ])
            .status()
            .context("yt-dlp failed")?;

        if !status.success() {
            return Err(anyhow::anyhow!("yt-dlp issue"));
        }
    }

    // Parsing loop
    let data = parse_segments(&conf, &video_name)?;

    // Write mapped OCR data
    let json_file_name = format!("{}-{}-{}.json", conf.org, conf.org_season, conf.org_xtra);
    fs::write(&json_file_name, serde_json::to_string_pretty(&data)?)?;
    println!("Finished writing to {}", json_file_name);

    Ok(())
}

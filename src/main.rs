use std::{fs, path::Path, process::Command};

use anyhow::{Context, Ok, Result};
use bwl_meta::constants::CONFIG_TOML;
use bwl_meta::models::Config;
use bwl_meta::parse_segments;

fn main() -> Result<()> {
    let conf: Config = toml::from_str(CONFIG_TOML)?;

    // Download yt video
    if !Path::new(&conf.video_name).exists() {
        let status = Command::new("yt-dlp")
            .args([
                "-f",
                "bestvideo[height<=1080]",
                "--no-playlist",
                "--no-audio",
                "-o",
                &conf.video_name,
                "--remux-video",
                "mp4",
                &conf.yt_url,
            ])
            .status()
            .context("yt-dlp")?;

        if !status.success() {
            return Err(anyhow::anyhow!("yt-dlp"));
        }
    }

    // Parsing loop
    let data = parse_segments(&conf)?;

    // Write extracted game data
    let json_file_name = format!("{} {} {}.json", conf.org, conf.org_season, conf.org_xtra);
    fs::write(&json_file_name, serde_json::to_string_pretty(&data)?)?;
    println!("JSON{}", json_file_name);

    Ok(())
}

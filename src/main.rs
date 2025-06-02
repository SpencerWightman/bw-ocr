mod constants;
mod helpers;
mod models;
use std::{collections::HashMap, fs, path::Path, process::Command};

use anyhow::{Context, Ok, Result, anyhow};
use constants::{CONFIG_TOML, FRAMES_DIR, OCR_DIR, ROIS};
use helpers::{parse_timestamp, supplies_diff};
use human_friendly_ids::Id;
use image::{DynamicImage, GenericImageView, ImageBuffer, Luma, imageops};
use leptess::{LepTess, Variable};
use models::{Config, MatchSegment, Roi, SupplyData};
use serde_json::{Map, Value, json};

fn main() -> Result<()> {
    let conf: Config = toml::from_str(CONFIG_TOML)?;
    fs::create_dir_all(FRAMES_DIR)?;
    fs::create_dir_all(OCR_DIR)?;
    let video_name = format!("{}.mp4", conf.yt_url.split_once('?').map(|x| x.1).unwrap());

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

fn parse_segments(conf: &Config, video_name: &str) -> Result<Map<String, Value>> {
    let mut output = Map::new();

    // Iterate through all matches specified in config.toml
    for (i, segment) in conf.matches.iter().enumerate() {
        let seg_name = format!("match{}", i + 1);
        let mut seg_map = Map::new();

        // Extract frames from the segment times and write
        extract_frames(segment, FRAMES_DIR, video_name)?;
        let mut match_map = parse_frames()?;

        // Map values for output
        seg_map.insert("player1".into(), json!(segment.player1));
        seg_map.insert("player2".into(), json!(segment.player2));
        seg_map.insert("race1".into(), json!(segment.race1));
        seg_map.insert("race2".into(), json!(segment.race2));
        seg_map.insert("dateTime".into(), json!(conf.date));
        seg_map.insert("org".into(), json!(conf.org));
        seg_map.insert("winner".into(), json!(segment.winner));
        seg_map.insert("orgSeason".into(), json!(conf.org_season));
        seg_map.insert("orgXtra".into(), json!(conf.org_xtra));

        // Remove ocr data that does not maintain for 2 seconds
        remove_inconsistent(&mut match_map);
        let value = serde_json::to_value(match_map)?;
        seg_map.insert("gameData".into(), value);

        output.insert(seg_name, Value::Object(seg_map));
    }

    Ok(output)
}

fn extract_frames(segment: &MatchSegment, frames_dir: &str, video_name: &str) -> Result<()> {
    // Get floating seconds from the config start/end times
    let start_s = parse_timestamp(&segment.start)?;
    let end_s = parse_timestamp(&segment.end)?;

    let write_path = format!("{}/frame_%04d.png", frames_dir);

    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-ss",
            &format!("{:.3}", start_s),
            "-to",
            &format!("{:.3}", end_s),
            "-i",
            video_name,
            "-vf",
            "fps=1",
            &write_path,
        ])
        .status()
        .context("Frame extraction failed")?;

    if !status.success() {
        return Err(anyhow::anyhow!("Frame extraction issue"));
    }

    Ok(())
}

fn parse_frames() -> Result<HashMap<String, SupplyData>> {
    let mut data_map: HashMap<String, SupplyData> = HashMap::new();

    // Iterate through the extracted frames
    for frame in fs::read_dir(FRAMES_DIR)? {
        let path = frame?.path();
        let img = image::open(&path)?;
        let timestamp = parse_text(&img, &ROIS[0])?;

        // This could happen if SOOP briefly switched to a shot of the crowd
        if timestamp.is_empty() {
            continue;
        }

        let player1_ocr_txt = parse_text(&img, &ROIS[1])?;
        let player2_ocr_txt = parse_text(&img, &ROIS[2])?;

        let supply_data = SupplyData {
            player1_supply: player1_ocr_txt,
            player2_supply: player2_ocr_txt,
        };

        data_map.insert(timestamp, supply_data);
    }

    Ok(data_map)
}

fn parse_text(img: &DynamicImage, roi: &Roi) -> Result<String> {
    let whitelist = match roi.name {
        "timestamp" => "0123456789:",
        _ => "0123456789/",
    };

    // Setup ocr
    let mut ocr = LepTess::new(None, "eng_best")?;
    ocr.set_variable(Variable::TesseditCharWhitelist, whitelist)?;

    let w = roi.width.min(img.width().saturating_sub(roi.x));
    let h = roi.height.min(img.height().saturating_sub(roi.y));

    // Crop, zoom, black-white
    let roi_crop = img.view(roi.x, roi.y, w, h).to_image();
    let enlarged_roi_crop =
        imageops::resize(&roi_crop, w * 4, h * 4, imageops::FilterType::Lanczos3);
    const THRESH: u8 = 130;
    let bw: ImageBuffer<Luma<u8>, _> = ImageBuffer::from_fn(
        enlarged_roi_crop.width(),
        enlarged_roi_crop.height(),
        |x, y| {
            let l = enlarged_roi_crop.get_pixel(x, y)[0];
            if l > THRESH { Luma([255]) } else { Luma([0]) }
        },
    );
    let id = Id::new(5);
    let path = format!("{}/{}_{}.png", &OCR_DIR, roi.name, id);
    bw.save(&path)?;

    ocr.set_image(&path)?;
    Ok(ocr.get_utf8_text()?.trim().to_string())
}

// Remove ocr data that is not consistent across 2 frames
fn remove_inconsistent(game_data: &mut HashMap<String, SupplyData>) {
    let mut inconsistent_game_data = Vec::new();
    let mut keys: Vec<_> = game_data.keys().cloned().collect();
    keys.sort();

    for (i, k) in keys.iter().enumerate() {
        if i + 2 > keys.len() {
            break;
        }

        let c_frame = &game_data[&keys[i]];
        let n_frame = &game_data[&keys[i + 1]];

        if supplies_diff(c_frame, n_frame) {
            inconsistent_game_data.push(k);
        }
    }

    for timestamp_key in inconsistent_game_data {
        game_data.remove(timestamp_key);
    }
}

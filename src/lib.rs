pub mod constants;
pub mod models;
use std::{collections::HashMap, fs, process::Command};

use anyhow::{Context, Ok, Result, anyhow};
pub use constants::{CONFIG_TOML, FRAMES_DIR, OCR_DIR, ROIS};
use human_friendly_ids::Id;
use image::{DynamicImage, GenericImageView, ImageBuffer, Luma, imageops};
use leptess::{LepTess, Variable};
pub use models::*;
use serde_json::{Map, Value, json};

pub fn parse_segments(conf: &Config, video_name: &str) -> Result<Map<String, Value>> {
    let mut output = Map::new();

    // Iterate through all matches specified in config.toml
    for (i, segment) in conf.matches.iter().enumerate() {
        let seg_name = format!("match{}", i + 1);
        let mut seg_map = Map::new();

        // Extract frames from the segment times and write
        extract_frames(segment, FRAMES_DIR, video_name)
            .context(format!("extract_frames failed: {seg_name}"))?;

        let match_map =
            parse_frames().context(format!("parse_frames for segment failed: {seg_name}"))?;

        // Map values for output
        seg_map.insert("player1".into(), json!(segment.player1));
        seg_map.insert("player2".into(), json!(segment.player2));
        seg_map.insert("race1".into(), json!(segment.race1));
        seg_map.insert("race2".into(), json!(segment.race2));
        seg_map.insert("dateTime".into(), json!(conf.date));
        seg_map.insert("org".into(), json!(conf.org));
        seg_map.insert("winner".into(), json!(segment.winner));
        seg_map.insert("orgSeason".into(), json!(conf.org_season));

        // Remove ocr data that does not maintain across prev or next frame
        let culled_match_map = remove_inconsistent(match_map)
            .context(format!("remove_inconsistent failed: {seg_name}"))?;

        let value = serde_json::to_value(culled_match_map)?;
        seg_map.insert("gameData".into(), value);

        output.insert(seg_name, Value::Object(seg_map));
    }

    Ok(output)
}

fn extract_frames(segment: &MatchSegment, frames_dir: &str, video_name: &str) -> Result<()> {
    // Get floating seconds from the config start/end times
    let start_s = parse_timestamp(&segment.start)
        .context(format!("parse_timestamp failed: {}", segment.start))?;

    let end_s = parse_timestamp(&segment.end)
        .context(format!("parse_timestamp failed: {}", segment.end))?;

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

pub fn parse_frames() -> Result<HashMap<String, SupplyData>> {
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

        let player1_ocr_txt =
            parse_text(&img, &ROIS[1]).context(format!("parse_text failed: {timestamp}"))?;

        let player2_ocr_txt =
            parse_text(&img, &ROIS[2]).context(format!("parse_text failed: {timestamp}"))?;

        let supply_data = SupplyData {
            player1supply: Some(player1_ocr_txt),
            player2supply: Some(player2_ocr_txt),
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
    ocr.set_variable(Variable::TesseditCharWhitelist, whitelist)
        .context(format!("ocr set variable failed: {}", roi.name))?;

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
    bw.save(&path)
        .context(format!("ocr bw save failed: {}", roi.name))?;

    ocr.set_image(&path)
        .context(format!("ocr set image failed: {}", roi.name))?;

    Ok(ocr.get_utf8_text()?.trim().to_string())
}

// Remove ocr data that is not consistent across 2 frames
pub fn remove_inconsistent(
    mut game_data: HashMap<String, SupplyData>,
) -> Result<HashMap<String, SupplyData>> {
    let mut keys: Vec<String> = game_data.keys().cloned().collect();
    keys.sort_unstable();

    for window in keys.windows(3) {
        let p_key = &window[0];
        let c_key = &window[1];
        let n_key = &window[2];

        let p_frame = &game_data[p_key];
        let c_frame = &game_data[c_key];
        let n_frame = &game_data[n_key];

        let (player1, player2) = find_consistent(p_frame, c_frame, n_frame)
            .context(format!("find_consistent failed: {}", c_key))?;

        if let Some(frame) = game_data.get_mut(c_key) {
            if !player1 {
                frame.player1supply = None;
            }
            if !player2 {
                frame.player2supply = None;
            }
        }
    }

    Ok(game_data)
}

pub fn parse_timestamp(tc: &str) -> Result<f64> {
    let parts: Vec<_> = tc.split(':').collect();
    if parts.len() != 3 {
        return Err(anyhow!("Invalid timecode: {}", tc));
    }
    let h: f64 = parts[0].parse()?;
    let m: f64 = parts[1].parse()?;
    let s: f64 = parts[2].parse()?;
    Ok(h * 3600.0 + m * 60.0 + s)
}

// Do values match next or previous
pub fn find_consistent(
    p_frame: &SupplyData,
    c_frame: &SupplyData,
    n_frame: &SupplyData,
) -> Result<(bool, bool)> {
    let c1 = normalize_slash(get_str(&c_frame.player1supply))?;
    let n1 = normalize_slash(get_str(&n_frame.player1supply))?;
    let p1 = normalize_slash(get_str(&p_frame.player1supply))?;

    let c2 = normalize_slash(get_str(&c_frame.player2supply))?;
    let n2 = normalize_slash(get_str(&n_frame.player2supply))?;
    let p2 = normalize_slash(get_str(&p_frame.player2supply))?;

    let player1 = (c1 == p1) || (c1 == n1);
    let player2 = (c2 == p2) || (c2 == n2);

    Ok((player1, player2))
}

fn get_str(supply_opt: &Option<String>) -> &str {
    supply_opt.as_deref().unwrap_or("0/0")
}

fn normalize_slash(txt: &str) -> Result<String> {
    Ok(txt
        .split('/')
        .map(str::trim)
        .collect::<Vec<&str>>()
        .join("/"))
}

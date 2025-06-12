pub mod constants;
pub mod models;
use std::{fs, process::Command};

use anyhow::{Context, Ok, Result, anyhow};
pub use constants::*;
use human_friendly_ids::Id;
use image::{DynamicImage, GenericImageView, ImageBuffer, Luma, imageops};
use leptess::{LepTess, Variable};
pub use models::*;

fn parse_timestamp(tc: &str) -> Result<usize> {
    let parts: Vec<_> = tc.split(':').collect();
    if parts.len() != 3 {
        return Err(anyhow!("Invalid timecode: {}", tc));
    }
    let h: usize = parts[0].parse()?;
    let m: usize = parts[1].parse()?;
    let s: usize = parts[2].parse()?;
    Ok(h * 3600 + m * 60 + s)
}

fn calc_seg_vec_len(seg_start: &str, seg_end: &str) -> Result<usize> {
    let end_s = parse_timestamp(seg_end)?;
    let start_s = parse_timestamp(seg_start)?;
    Ok(end_s - start_s)
}

pub fn parse_segments<'a>(conf: &'a Config, video_name: &str) -> Result<Vec<EntryData<'a>>> {
    let output_len = conf.matches.len();
    let mut output: Vec<EntryData> = Vec::with_capacity(output_len);

    // Iterate through all matches specified in config.toml
    for segment in &conf.matches {
        let seg_vec_len = calc_seg_vec_len(&segment.start, &segment.end)?;
        let seg_vec: Vec<FrameData> = Vec::with_capacity(seg_vec_len);

        // Extract frames from the segment times and write
        extract_frames(segment, FRAMES_DIR, video_name)?;

        // Add ocr data
        let populated_seg_vec = parse_frames(seg_vec)?;

        // Remove ocr data that does not maintain across prev or next frame
        let validated_seg_vec = remove_inconsistent(populated_seg_vec)?;

        let entry = EntryData {
            segment,
            ocr: validated_seg_vec,
        };
        output.push(entry);
    }

    Ok(output)
}

fn extract_frames(segment: &ConfigMatchSegment, frames_dir: &str, video_name: &str) -> Result<()> {
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

pub fn parse_frames(mut seg_vec: Vec<FrameData>) -> Result<Vec<FrameData>> {
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

        let parsed_data = FrameData {
            timestamp,
            player1supply: Some(player1_ocr_txt),
            player2supply: Some(player2_ocr_txt),
        };

        seg_vec.push(parsed_data);
    }

    Ok(seg_vec)
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
pub fn remove_inconsistent(mut seg_vec: Vec<FrameData>) -> Result<Vec<FrameData>> {
    let len = seg_vec.len();
    for i in 1..len - 1 {
        let p_frame = &seg_vec[0];
        let c_frame = &seg_vec[i];
        let n_frame = &seg_vec[i + 1];

        let (player1_valid, player2_valid) =
            find_consistent(p_frame, c_frame, n_frame).context(format!(
                "remove_inconsistent failed at timestamp: {}",
                c_frame.timestamp
            ))?;

        if !player1_valid {
            seg_vec[i].player1supply = None;
        }
        if !player2_valid {
            seg_vec[i].player2supply = None;
        }
    }
    Ok(seg_vec)
}

// Do values match next or previous
fn find_consistent(
    p_frame: &FrameData,
    c_frame: &FrameData,
    n_frame: &FrameData,
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

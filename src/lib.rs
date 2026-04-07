pub mod constants;
pub mod models;
pub mod recognizer;
use std::time::Duration;

use anyhow::{Context, Ok, Result, anyhow};
pub use constants::*;
use image::DynamicImage;
use pixelpipe::{
    config::Roi as PixelPipeRoi,
    crop,
    ffmpeg::ffmpeg_frames,
    preprocess,
};
pub use models::*;
use recognizer::recognize_text;

fn parse_timestamp(timestamp: &str) -> Result<usize> {
    let parts: Vec<_> = timestamp.split(':').collect();
    if parts.len() != 3 {
        return Err(anyhow!("Invalid timestamp: {}", timestamp));
    }
    let h: usize = parts[0].parse()?;
    let m: usize = parts[1].parse()?;
    let s: usize = parts[2].parse()?;
    Ok(h * 3600 + m * 60 + s)
}

pub fn parse_segments<'a>(conf: &'a Config) -> Result<Vec<EntryData<'a>>> {
    let output_len = conf.matches.len();
    let mut output: Vec<EntryData> = Vec::with_capacity(output_len);

    for segment in &conf.matches {
        let start_s = parse_timestamp(&segment.start)?;
        let end_s = parse_timestamp(&segment.end)?;
        let seg_vec_len = end_s - start_s;
        let mut seg_vec: Vec<FrameData> = Vec::with_capacity(seg_vec_len);
        parse_segment(&mut seg_vec, seg_vec_len, start_s, end_s, &conf.video_name)?;
        nullify_inconsistent(&mut seg_vec)?;
        output.push(EntryData {
            segment,
            ocr: seg_vec,
        });
    }

    Ok(output)
}

pub fn parse_segment(
    seg_vec: &mut Vec<FrameData>,
    seg_vec_len: usize,
    start_s: usize,
    end_s: usize,
    video_name: &str,
) -> Result<()> {
    let ranges = [Duration::from_secs(start_s as u64)..=Duration::from_secs(end_s as u64)];
    let mut frames = ffmpeg_frames(video_name, 1, Some(&ranges)).context("ffmpeg decode")?;

    for frame in frames.by_ref().take(seg_vec_len) {
        let frame = frame?;
        let img = frame.image;

        let timestamp = parse_text(&img, &ROIS[0]);
        if timestamp.is_empty() {
            continue;
        }

        let player1_ocr = parse_text(&img, &ROIS[1]);
        let player2_ocr = parse_text(&img, &ROIS[2]);

        seg_vec.push(FrameData {
            timestamp,
            player1supply: Some(player1_ocr),
            player2supply: Some(player2_ocr),
        });
    }

    Ok(())
}

fn parse_text(img: &DynamicImage, roi: &Roi) -> String {
    let roi = roi_to_pixelpipe(roi);
    let roi_crop = crop::crop_image(img, &roi);
    let enlarged = preprocess::resize_luma(
        &preprocess::to_luma(&roi_crop),
        roi.width * 4,
        roi.height * 4,
    );
    let bw = preprocess::threshold_luma(&enlarged, THRESH);
    recognize_text(roi.name.as_deref().unwrap_or(""), &bw)
}

fn roi_to_pixelpipe(roi: &Roi) -> PixelPipeRoi {
    PixelPipeRoi {
        name: Some(roi.name.to_string()),
        x: roi.x,
        y: roi.y,
        width: roi.width,
        height: roi.height,
    }
}

pub fn nullify_inconsistent(seg_vec: &mut [FrameData]) -> Result<()> {
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
    Ok(())
}

fn find_consistent(
    p_frame: &FrameData,
    c_frame: &FrameData,
    n_frame: &FrameData,
) -> Result<(bool, bool)> {
    let c1 = &c_frame.player1supply;
    let n1 = &n_frame.player1supply;
    let p1 = &p_frame.player1supply;

    let c2 = &c_frame.player2supply;
    let n2 = &n_frame.player2supply;
    let p2 = &p_frame.player2supply;

    let player1 = (c1 == p1) || (c1 == n1);
    let player2 = (c2 == p2) || (c2 == n2);

    Ok((player1, player2))
}

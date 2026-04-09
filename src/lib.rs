pub mod constants;
pub mod models;
pub mod recognizer;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Ok, Result, anyhow};
pub use constants::*;
use image::{DynamicImage, GrayImage};
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
    parse_segments_with_progress(conf, |_| {})
}

pub fn parse_segments_with_progress<'a, F>(
    conf: &'a Config,
    mut on_frame: F,
) -> Result<Vec<EntryData<'a>>>
where
    F: FnMut(usize),
{
    let output_len = conf.matches.len();
    let mut output: Vec<EntryData> = Vec::with_capacity(output_len);

    for (segment_idx, segment) in conf.matches.iter().enumerate() {
        let start_s = parse_timestamp(&segment.start)?;
        let end_s = parse_timestamp(&segment.end)?;
        let seg_vec_len = end_s - start_s;
        let mut seg_vec: Vec<FrameData> = Vec::with_capacity(seg_vec_len);
        parse_segment(&mut seg_vec, seg_vec_len, start_s, end_s, &conf.video_name, || {
            on_frame(segment_idx);
        })?;
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
    mut on_frame: impl FnMut(),
) -> Result<()> {
    let ranges = [Duration::from_secs(start_s as u64)..=Duration::from_secs(end_s as u64)];
    let mut frames = ffmpeg_frames(video_name, 1, Some(&ranges)).context("ffmpeg decode")?;

    for frame in frames.by_ref().take(seg_vec_len) {
        let frame = frame?;
        let img = frame.image;
        on_frame();

        if let Some(frame_data) = extract_frame_data(&img) {
            seg_vec.push(frame_data);
        }
    }

    Ok(())
}

pub fn parse_frame(video_name: &str, second: usize) -> Result<Option<FrameData>> {
    let ranges = [single_frame_range(second)];
    let mut frames = ffmpeg_frames(video_name, 1, Some(&ranges)).context("ffmpeg decode")?;
    let Some(frame) = frames.next() else {
        return Ok(None);
    };
    let frame = frame?;
    Ok(extract_frame_data(&frame.image))
}

pub fn dump_frame_debug(video_name: &str, second: usize, out_dir: &Path) -> Result<Option<FrameData>> {
    let ranges = [single_frame_range(second)];
    let mut frames = ffmpeg_frames(video_name, 1, Some(&ranges)).context("ffmpeg decode")?;
    let Some(frame) = frames.next() else {
        return Ok(None);
    };
    let frame = frame?;
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("creating {}", out_dir.display()))?;

    let mut frame_data = FrameData {
        timestamp: String::new(),
        player1supply: None,
        player2supply: None,
    };

    for roi in &ROIS {
        let prepared = prepare_roi(&frame.image, roi);
        prepared
            .crop
            .save(out_dir.join(format!("{}-crop.png", roi.name)))
            .with_context(|| format!("writing {} crop", roi.name))?;
        prepared
            .bw
            .save(out_dir.join(format!("{}-bw.png", roi.name)))
            .with_context(|| format!("writing {} bw", roi.name))?;

        match roi.name {
            "timestamp" => frame_data.timestamp = prepared.text,
            "player1supply" => frame_data.player1supply = none_if_empty(prepared.text),
            "player2supply" => frame_data.player2supply = none_if_empty(prepared.text),
            _ => {}
        }
    }

    std::fs::write(
        out_dir.join("ocr.json"),
        serde_json::to_string_pretty(&frame_data)?,
    )
    .with_context(|| format!("writing {}", out_dir.join("ocr.json").display()))?;

    if frame_data.timestamp.is_empty() {
        return Ok(None);
    }

    Ok(Some(frame_data))
}

fn single_frame_range(second: usize) -> std::ops::RangeInclusive<Duration> {
    Duration::from_secs(second as u64)..=Duration::from_secs(second as u64 + 1)
}

fn extract_frame_data(img: &DynamicImage) -> Option<FrameData> {
    let timestamp = parse_text(img, &ROIS[0]);
    if timestamp.is_empty() {
        return None;
    }

    let player1_ocr = parse_text(img, &ROIS[1]);
    let player2_ocr = parse_text(img, &ROIS[2]);

    Some(FrameData {
        timestamp,
        player1supply: none_if_empty(player1_ocr),
        player2supply: none_if_empty(player2_ocr),
    })
}

fn parse_text(img: &DynamicImage, roi: &Roi) -> String {
    prepare_roi(img, roi).text
}

struct PreparedRoi {
    crop: DynamicImage,
    bw: GrayImage,
    text: String,
}

fn prepare_roi(img: &DynamicImage, roi: &Roi) -> PreparedRoi {
    let roi = roi_to_pixelpipe(roi);
    let roi_crop = crop::crop_image(img, &roi);
    let enlarged = preprocess::resize_luma(
        &preprocess::to_luma(&roi_crop),
        roi.width * 4,
        roi.height * 4,
    );
    let bw = preprocess::threshold_luma(&enlarged, THRESH);
    let text = recognize_text(roi.name.as_deref().unwrap_or(""), &bw);

    PreparedRoi {
        crop: roi_crop,
        bw,
        text,
    }
}

fn none_if_empty(text: String) -> Option<String> {
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
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
    if len < 3 {
        return Ok(());
    }

    for i in 1..len - 1 {
        let p_frame = &seg_vec[i - 1];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nullify_inconsistent_uses_previous_frame() {
        let mut frames = vec![
            FrameData {
                timestamp: "00:57".to_string(),
                player1supply: Some("9/9".to_string()),
                player2supply: Some("9/9".to_string()),
            },
            FrameData {
                timestamp: "00:58".to_string(),
                player1supply: Some("10/9".to_string()),
                player2supply: Some("10/9".to_string()),
            },
            FrameData {
                timestamp: "00:59".to_string(),
                player1supply: Some("10/9".to_string()),
                player2supply: Some("10/9".to_string()),
            },
        ];

        nullify_inconsistent(&mut frames).unwrap();

        assert_eq!(frames[1].player1supply.as_deref(), Some("10/9"));
        assert_eq!(frames[1].player2supply.as_deref(), Some("10/9"));
    }
}

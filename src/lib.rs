pub mod constants;
pub mod models;
use std::{
    io::{BufReader, Cursor},
    process::{Command, Stdio},
};

use anyhow::{Context, Ok, Result, anyhow};
pub use constants::*;
use image::ImageDecoder;
use image::codecs::pnm::PnmDecoder;
use image::{DynamicImage, GenericImageView, ImageBuffer, ImageOutputFormat, Luma, imageops};
use leptess::{LepTess, Variable};
pub use models::*;

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
    let mut ocr = LepTess::new(None, "eng_best").context("Tesseract init")?;
    ocr.set_variable(Variable::TesseditPagesegMode, "8")
        .context("page_seg_mode")?;
    let output_len = conf.matches.len();
    let mut output: Vec<EntryData> = Vec::with_capacity(output_len);

    // Iterate through all matches specified in config.toml
    for segment in &conf.matches {
        let start_s = parse_timestamp(&segment.start)?;
        let end_s = parse_timestamp(&segment.end)?;
        let seg_vec_len = end_s - start_s;
        let mut seg_vec: Vec<FrameData> = Vec::with_capacity(seg_vec_len);
        parse_segment(
            &mut seg_vec,
            &mut ocr,
            seg_vec_len,
            start_s,
            end_s,
            &conf.video_name,
        )?;
        nullify_inconsistent(&mut seg_vec)?;
        let entry = EntryData {
            segment,
            ocr: seg_vec,
        };
        output.push(entry);
    }

    Ok(output)
}

pub fn parse_segment(
    seg_vec: &mut Vec<FrameData>,
    ocr: &mut LepTess,
    seg_vec_len: usize,
    start_s: usize,
    end_s: usize,
    video_name: &str,
) -> Result<()> {
    let mut ffmpeg = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &format!("{start_s}"),
            "-to",
            &format!("{end_s}"),
            "-i",
            video_name,
            "-vf",
            "fps=1",
            "-f",
            "image2pipe",
            "-vcodec",
            "ppm",
            "-",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .context("ffmpeg spawn")?;

    let stdout = ffmpeg.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    // Loop the number of seconds in the segment
    for _ in 0..seg_vec_len {
        let decoder = PnmDecoder::new(&mut reader)?;
        let (w, h) = decoder.dimensions();
        let ctype = decoder.color_type();
        let bpp = ctype.bytes_per_pixel();

        let mut raw = vec![0u8; w as usize * h as usize * bpp as usize];
        decoder.read_image(&mut raw)?;

        let img: DynamicImage =
            DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, raw).expect("wrong size"));

        let timestamp = parse_text(&img, &ROIS[0], ocr)?;
        if timestamp.is_empty() {
            continue;
        }

        let player1_ocr =
            parse_text(&img, &ROIS[1], ocr).context(format!("parse_text failed: {}", timestamp))?;
        let player2_ocr =
            parse_text(&img, &ROIS[2], ocr).context(format!("parse_text failed: {}", timestamp))?;

        seg_vec.push(FrameData {
            timestamp,
            player1supply: Some(player1_ocr),
            player2supply: Some(player2_ocr),
        });
    }

    let status = ffmpeg.wait()?;
    if !status.success() {
        Err(anyhow::anyhow!("ffmpeg exit"))
    } else {
        Ok(())
    }
}

fn parse_text(img: &DynamicImage, roi: &Roi, ocr: &mut LepTess) -> Result<String> {
    let whitelist = match roi.name {
        "timestamp" => "0123456789:",
        _ => "0123456789/",
    };

    // Setup ocr
    ocr.set_variable(Variable::TesseditCharWhitelist, whitelist)
        .context(format!("Ocr set variable failed: {}", roi.name))?;

    // Crop, zoom, black-white
    let roi_crop = img.view(roi.x, roi.y, roi.width, roi.height).to_image();
    let enlarged_roi_crop = imageops::resize(
        &roi_crop,
        roi.width * 4,
        roi.height * 4,
        imageops::FilterType::Lanczos3,
    );
    let bw: ImageBuffer<Luma<u8>, _> = ImageBuffer::from_fn(
        enlarged_roi_crop.width(),
        enlarged_roi_crop.height(),
        |x, y| {
            let l = enlarged_roi_crop.get_pixel(x, y)[0];
            if l > THRESH { Luma([255]) } else { Luma([0]) }
        },
    );

    // Cursor to keep in memory -- new Vec should have capacity
    let mut cursor = Cursor::new(Vec::new());
    bw.write_to(&mut cursor, ImageOutputFormat::Png)
        .context("Encoding ROI to PNG")?;

    let png_data = cursor.into_inner();
    ocr.set_image_from_mem(&png_data)
        .context(format!("Ocr set image failed: {}", roi.name))?;

    let raw = ocr.get_utf8_text()?;
    let cleaned = raw
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    Ok(cleaned)
}

// Remove ocr data that is not consistent across 2 frames
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

// Do values match next or previous
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

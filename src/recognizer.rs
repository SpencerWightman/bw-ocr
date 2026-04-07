use image::{GrayImage, Luma, imageops};
use std::sync::OnceLock;

const GLYPH_WIDTH: u32 = 12;
const GLYPH_HEIGHT: u32 = 20;

pub fn recognize_text(roi_name: &str, image: &GrayImage) -> String {
    let allowed = match roi_name {
        "timestamp" => "0123456789:",
        _ => "0123456789/",
    };

    segment_glyphs(image)
        .into_iter()
        .filter_map(|glyph| recognize_glyph(&glyph, allowed))
        .collect()
}

fn segment_glyphs(image: &GrayImage) -> Vec<GrayImage> {
    let mut ranges = Vec::new();
    let mut start = None;
    let mut gap = 0u32;

    for x in 0..image.width() {
        if column_has_ink(image, x) {
            if start.is_none() {
                start = Some(x);
            }
            gap = 0;
        } else if let Some(run_start) = start {
            gap += 1;
            if gap > 1 {
                ranges.push((run_start, x - gap));
                start = None;
                gap = 0;
            }
        }
    }

    if let Some(run_start) = start {
        ranges.push((run_start, image.width().saturating_sub(1)));
    }

    ranges
        .into_iter()
        .filter_map(|(left, right)| crop_to_ink(image, left, right))
        .collect()
}

fn column_has_ink(image: &GrayImage, x: u32) -> bool {
    (0..image.height()).any(|y| image.get_pixel(x, y).0[0] > 0)
}

fn row_has_ink(image: &GrayImage, y: u32, left: u32, right: u32) -> bool {
    (left..=right).any(|x| image.get_pixel(x, y).0[0] > 0)
}

fn crop_to_ink(image: &GrayImage, left: u32, right: u32) -> Option<GrayImage> {
    let top = (0..image.height()).find(|&y| row_has_ink(image, y, left, right))?;
    let bottom = (0..image.height())
        .rev()
        .find(|&y| row_has_ink(image, y, left, right))?;

    Some(imageops::crop_imm(image, left, top, right - left + 1, bottom - top + 1).to_image())
}

fn recognize_glyph(image: &GrayImage, allowed: &str) -> Option<char> {
    if allowed.contains(':') && looks_like_colon(image) {
        return Some(':');
    }

    let normalized = normalize(image);
    allowed
        .chars()
        .filter(|&ch| ch != ':')
        .map(|ch| (ch, glyph_score(&normalized, template_for(ch))))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(ch, _)| ch)
}

fn normalize(image: &GrayImage) -> GrayImage {
    let resized = imageops::resize(image, GLYPH_WIDTH, GLYPH_HEIGHT, imageops::FilterType::Nearest);
    GrayImage::from_fn(GLYPH_WIDTH, GLYPH_HEIGHT, |x, y| {
        if resized.get_pixel(x, y).0[0] > 127 {
            Luma([255])
        } else {
            Luma([0])
        }
    })
}

fn glyph_score(candidate: &GrayImage, template: &GrayImage) -> f32 {
    let mut matches = 0u32;
    for y in 0..GLYPH_HEIGHT {
        for x in 0..GLYPH_WIDTH {
            let a = candidate.get_pixel(x, y).0[0] > 0;
            let b = template.get_pixel(x, y).0[0] > 0;
            if a == b {
                matches += 1;
            }
        }
    }
    matches as f32 / (GLYPH_WIDTH * GLYPH_HEIGHT) as f32
}

fn looks_like_colon(image: &GrayImage) -> bool {
    let runs = row_runs(image);
    runs.len() == 2 && image.width() * 4 <= image.height() * 3
}

fn row_runs(image: &GrayImage) -> Vec<(u32, u32)> {
    let mut runs = Vec::new();
    let mut start = None;

    for y in 0..image.height() {
        if (0..image.width()).any(|x| image.get_pixel(x, y).0[0] > 0) {
            if start.is_none() {
                start = Some(y);
            }
        } else if let Some(run_start) = start {
            runs.push((run_start, y - 1));
            start = None;
        }
    }

    if let Some(run_start) = start {
        runs.push((run_start, image.height().saturating_sub(1)));
    }

    runs
}

fn template_for(ch: char) -> &'static GrayImage {
    static CACHE: OnceLock<Vec<(char, GrayImage)>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        GLYPH_PATTERNS
            .iter()
            .map(|(glyph, rows)| (*glyph, render_template(rows)))
            .collect()
    });

    cache
        .iter()
        .find(|(glyph, _)| *glyph == ch)
        .map(|(_, image)| image)
        .expect("missing glyph template")
}

fn render_template(rows: &[&str]) -> GrayImage {
    let height = rows.len() as u32;
    let width = rows[0].len() as u32;
    let glyph = GrayImage::from_fn(width, height, |x, y| {
        let pixel = rows[y as usize].as_bytes()[x as usize];
        if pixel == b'#' {
            Luma([255])
        } else {
            Luma([0])
        }
    });
    normalize(&glyph)
}

const GLYPH_PATTERNS: &[(char, &[&str])] = &[
    ('0', &[".###.", "##.##", "##.##", "##.##", "##.##", "##.##", ".###."]),
    ('1', &["..##.", ".###.", "..##.", "..##.", "..##.", "..##.", ".####"]),
    ('2', &[".###.", "##.##", "...##", "..##.", ".##..", "##...", "#####"]),
    ('3', &["####.", "...##", "..##.", "..###", "...##", "##.##", ".###."]),
    ('4', &["...##", "..###", ".####", "##.##", "#####", "...##", "...##"]),
    ('5', &["#####", "##...", "####.", "...##", "...##", "##.##", ".###."]),
    ('6', &[".###.", "##.##", "##...", "####.", "##.##", "##.##", ".###."]),
    ('7', &["#####", "...##", "..##.", ".##..", ".##..", ".##..", ".##.."]),
    ('8', &[".###.", "##.##", "##.##", ".###.", "##.##", "##.##", ".###."]),
    ('9', &[".###.", "##.##", "##.##", ".####", "...##", "##.##", ".###."]),
    (':', &[".....", "..#..", ".....", ".....", "..#..", ".....", "....."]),
    ('/', &["....#", "...##", "...#.", "..##.", ".##..", ".#...", "##..."]),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn render_char(ch: char) -> GrayImage {
        template_for(ch).clone()
    }

    #[test]
    fn recognizes_timestamp_tokens() {
        let glyphs = ['1', '2', ':', '3', '4'];
        let width = GLYPH_WIDTH as usize * glyphs.len() + (glyphs.len() - 1) * 2;
        let mut image = GrayImage::new(width as u32, GLYPH_HEIGHT);
        let mut x = 0u32;
        for ch in glyphs {
            let glyph = render_char(ch);
            imageops::overlay(&mut image, &glyph, x.into(), 0);
            x += GLYPH_WIDTH + 2;
        }

        assert_eq!(recognize_text("timestamp", &image), "12:34");
    }

    #[test]
    fn recognizes_supply_tokens() {
        let glyphs = ['4', '2', '/', '5', '8'];
        let width = GLYPH_WIDTH as usize * glyphs.len() + (glyphs.len() - 1) * 2;
        let mut image = GrayImage::new(width as u32, GLYPH_HEIGHT);
        let mut x = 0u32;
        for ch in glyphs {
            let glyph = render_char(ch);
            imageops::overlay(&mut image, &glyph, x.into(), 0);
            x += GLYPH_WIDTH + 2;
        }

        assert_eq!(recognize_text("player1supply", &image), "42/58");
    }
}

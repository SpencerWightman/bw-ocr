use image::{GrayImage, Luma, imageops};
use std::{
    fs,
    path::Path,
    sync::OnceLock,
};
#[cfg(not(test))]
use std::path::PathBuf;

const GLYPH_WIDTH: u32 = 12;
const GLYPH_HEIGHT: u32 = 20;
#[cfg(not(test))]
const DEFAULT_TEMPLATE_DIR: &str = "templates/glyphs";

pub fn recognize_text(roi_name: &str, image: &GrayImage) -> String {
    let roi_group = roi_group(roi_name);
    let glyphs = segment_glyphs(image);
    let ranked: Vec<DebugGlyphScores> = glyphs
        .iter()
        .map(|glyph| score_glyph_for_roi(glyph, roi_group, 10))
        .collect();

    decode_text(roi_name, &ranked).unwrap_or_default()
}

pub fn debug_segment_glyphs(image: &GrayImage) -> Vec<GrayImage> {
    segment_glyphs(image)
}

#[derive(Debug, Clone)]
pub struct GlyphScore {
    pub ch: char,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct DebugGlyphScores {
    pub best: Option<char>,
    pub accepted: bool,
    pub scores: Vec<GlyphScore>,
}

pub fn debug_score_glyphs(roi_name: &str, image: &GrayImage, top_n: usize) -> Vec<DebugGlyphScores> {
    let roi_group = roi_group(roi_name);
    segment_glyphs(image)
        .into_iter()
        .map(|glyph| score_glyph_for_roi(&glyph, roi_group, top_n))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoiGroup {
    Timestamp,
    Supply,
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

fn glyph_score_for_char(candidate: &GrayImage, ch: char, roi_group: RoiGroup) -> f32 {
    templates_for(roi_group, ch)
        .iter()
        .map(|template| glyph_score(candidate, template))
        .fold(0.0, f32::max)
}

fn score_glyph_for_roi(image: &GrayImage, roi_group: RoiGroup, top_n: usize) -> DebugGlyphScores {
    let allowed = allowed_chars(roi_group);
    let normalized = normalize(image);
    let mut scores: Vec<GlyphScore> = allowed
        .chars()
        .map(|ch| GlyphScore {
            ch,
            score: glyph_score_for_char(&normalized, ch, roi_group),
        })
        .collect();
    scores.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    let best = scores.first().map(|entry| entry.ch);
    let accepted = scores
        .first()
        .map(|entry| entry.score >= min_glyph_score(roi_group))
        .unwrap_or(false);
    let keep = top_n.max(1).min(scores.len());
    scores.truncate(keep);

    DebugGlyphScores {
        best,
        accepted,
        scores,
    }
}

fn allowed_chars(roi_group: RoiGroup) -> &'static str {
    match roi_group {
        RoiGroup::Timestamp => "0123456789",
        RoiGroup::Supply => "0123456789/",
    }
}

fn roi_group(roi_name: &str) -> RoiGroup {
    match roi_name {
        "timestamp" => RoiGroup::Timestamp,
        _ => RoiGroup::Supply,
    }
}

fn min_glyph_score(roi_group: RoiGroup) -> f32 {
    match roi_group {
        RoiGroup::Timestamp => 0.60,
        RoiGroup::Supply => 0.64,
    }
}

fn decode_text(roi_name: &str, glyphs: &[DebugGlyphScores]) -> Option<String> {
    match roi_name {
        "timestamp" => decode_timestamp(glyphs),
        _ => decode_supply(glyphs),
    }
}

fn decode_timestamp(glyphs: &[DebugGlyphScores]) -> Option<String> {
    if glyphs.len() != 5 {
        return None;
    }

    let first = choose_best_digit(&glyphs[0], &['0', '1', '2', '3'])?;
    let second = choose_best_digit(&glyphs[1], &['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'])?;
    let third = ':';
    let fourth = choose_best_digit(&glyphs[3], &['0', '1', '2', '3', '4', '5'])?;
    let fifth = choose_best_digit(&glyphs[4], &['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'])?;

    let seconds = format!("{fourth}{fifth}");
    match seconds.parse::<u8>() {
        Ok(value) if value < 60 => Some(format!("{first}{second}{third}{fourth}{fifth}")),
        _ => None,
    }
}

fn decode_supply(glyphs: &[DebugGlyphScores]) -> Option<String> {
    match glyphs.len() {
        3 => decode_supply_pattern(glyphs, &[false, true, false]),
        4 => {
            let left = decode_supply_pattern(glyphs, &[false, true, false, false]);
            let right = decode_supply_pattern(glyphs, &[false, false, true, false]);
            choose_better_supply(left, right)
        }
        5 => decode_supply_pattern(glyphs, &[false, false, true, false, false]),
        _ => None,
    }
}

fn decode_supply_pattern(glyphs: &[DebugGlyphScores], slash_positions: &[bool]) -> Option<String> {
    if glyphs.len() != slash_positions.len() {
        return None;
    }

    let mut output = String::new();
    for (glyph, is_slash) in glyphs.iter().zip(slash_positions.iter().copied()) {
        if is_slash {
            let slash_score = score_for_char(glyph, '/').unwrap_or(0.0);
            if slash_score < min_glyph_score(RoiGroup::Supply) {
                return None;
            }
            output.push('/');
        } else {
            output.push(choose_best_digit(glyph, &['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'])?);
        }
    }

    validate_supply(&output)
}

fn choose_better_supply(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (Some(a), Some(_b)) => Some(a),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn validate_supply(text: &str) -> Option<String> {
    parse_supply_numbers(text)?;
    Some(text.to_string())
}

fn parse_supply_numbers(text: &str) -> Option<(u16, u16)> {
    let parts: Vec<_> = text.split('/').collect();
    if parts.len() != 2 {
        return None;
    }
    let used: u16 = parts[0].parse().ok()?;
    let cap: u16 = parts[1].parse().ok()?;
    Some((used, cap))
}

fn choose_best_digit(glyph: &DebugGlyphScores, allowed: &[char]) -> Option<char> {
    glyph
        .scores
        .iter()
        .filter(|entry| allowed.contains(&entry.ch))
        .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap())
        .map(|entry| entry.ch)
}

fn score_for_char(glyph: &DebugGlyphScores, ch: char) -> Option<f32> {
    glyph
        .scores
        .iter()
        .find(|entry| entry.ch == ch)
        .map(|entry| entry.score)
}

fn templates_for(roi_group: RoiGroup, ch: char) -> &'static [GrayImage] {
    template_cache(roi_group)
        .iter()
        .find(|(glyph, _)| *glyph == ch)
        .map(|(_, images)| images.as_slice())
        .expect("missing glyph template")
}

fn template_cache(roi_group: RoiGroup) -> &'static Vec<(char, Vec<GrayImage>)> {
    static TIMESTAMP_CACHE: OnceLock<Vec<(char, Vec<GrayImage>)>> = OnceLock::new();
    static SUPPLY_CACHE: OnceLock<Vec<(char, Vec<GrayImage>)>> = OnceLock::new();

    match roi_group {
        RoiGroup::Timestamp => TIMESTAMP_CACHE.get_or_init(|| template_set_for_runtime(RoiGroup::Timestamp)),
        RoiGroup::Supply => SUPPLY_CACHE.get_or_init(|| template_set_for_runtime(RoiGroup::Supply)),
    }
}

#[cfg(not(test))]
fn load_template_set(roi_group: RoiGroup) -> Vec<(char, Vec<GrayImage>)> {
    let mut templates = built_in_templates(roi_group);
    if let Ok(real_templates) = load_templates_from_dir(&template_dir(), roi_group) {
        merge_templates(&mut templates, real_templates);
    }
    templates
}

#[cfg(not(test))]
fn template_set_for_runtime(roi_group: RoiGroup) -> Vec<(char, Vec<GrayImage>)> {
    load_template_set(roi_group)
}

#[cfg(test)]
fn template_set_for_runtime(roi_group: RoiGroup) -> Vec<(char, Vec<GrayImage>)> {
    built_in_templates(roi_group)
}

#[cfg(not(test))]
fn template_dir() -> PathBuf {
    std::env::var_os("BWL_TEMPLATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_TEMPLATE_DIR))
}

#[cfg(not(test))]
fn merge_templates(base: &mut Vec<(char, Vec<GrayImage>)>, extra: Vec<(char, Vec<GrayImage>)>) {
    for (glyph, mut images) in extra {
        if let Some((_, existing)) = base.iter_mut().find(|(existing_glyph, _)| *existing_glyph == glyph) {
            images.append(existing);
            *existing = images;
        } else {
            base.push((glyph, images));
        }
    }
}

fn built_in_templates(roi_group: RoiGroup) -> Vec<(char, Vec<GrayImage>)> {
    glyph_patterns(roi_group)
        .iter()
        .map(|(glyph, rows)| (*glyph, vec![render_template(rows)]))
        .collect()
}

fn load_templates_from_dir(
    path: &Path,
    roi_group: RoiGroup,
) -> Result<Vec<(char, Vec<GrayImage>)>, std::io::Error> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let group_dir = path.join(roi_group_dir_name(roi_group));
    if !group_dir.exists() {
        return Ok(Vec::new());
    }

    let mut output = Vec::new();
    for glyph in supported_glyphs(roi_group) {
        let glyph_dir = group_dir.join(glyph_dir_name(glyph));
        if !glyph_dir.exists() {
            continue;
        }

        let mut images = Vec::new();
        for entry in fs::read_dir(&glyph_dir)? {
            let entry = entry?;
            let entry_path = entry.path();
            if !is_png_path(&entry_path) {
                continue;
            }

            if let Ok(image) = image::open(&entry_path) {
                images.push(normalize(&image.to_luma8()));
            }
        }

        if !images.is_empty() {
            output.push((glyph, images));
        }
    }

    Ok(output)
}

fn supported_glyphs(roi_group: RoiGroup) -> impl Iterator<Item = char> {
    allowed_chars(roi_group).chars()
}

fn roi_group_dir_name(roi_group: RoiGroup) -> &'static str {
    match roi_group {
        RoiGroup::Timestamp => "timestamp",
        RoiGroup::Supply => "supply",
    }
}

fn glyph_dir_name(ch: char) -> &'static str {
    match ch {
        '0' => "0",
        '1' => "1",
        '2' => "2",
        '3' => "3",
        '4' => "4",
        '5' => "5",
        '6' => "6",
        '7' => "7",
        '8' => "8",
        '9' => "9",
        '/' => "slash",
        ':' => "colon",
        _ => panic!("unsupported glyph"),
    }
}

fn is_png_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("png"))
        .unwrap_or(false)
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

fn glyph_patterns(roi_group: RoiGroup) -> &'static [(char, &'static [&'static str])] {
    match roi_group {
        RoiGroup::Timestamp => TIMESTAMP_GLYPH_PATTERNS,
        RoiGroup::Supply => SUPPLY_GLYPH_PATTERNS,
    }
}

const TIMESTAMP_GLYPH_PATTERNS: &[(char, &[&str])] = &[
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
];

const SUPPLY_GLYPH_PATTERNS: &[(char, &[&str])] = &[
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
    ('/', &["....#", "...##", "...#.", "..##.", ".##..", ".#...", "##..."]),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn render_char(roi_group: RoiGroup, ch: char) -> GrayImage {
        if ch == ':' {
            return render_template(&[".....", "..#..", ".....", ".....", "..#..", ".....", "....."]);
        }
        templates_for(roi_group, ch)[0].clone()
    }

    #[test]
    fn recognizes_timestamp_tokens() {
        let glyphs = ['1', '2', ':', '3', '4'];
        let width = GLYPH_WIDTH as usize * glyphs.len() + (glyphs.len() - 1) * 2;
        let mut image = GrayImage::new(width as u32, GLYPH_HEIGHT);
        let mut x = 0u32;
        for ch in glyphs {
            let glyph = render_char(RoiGroup::Timestamp, ch);
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
            let glyph = render_char(RoiGroup::Supply, ch);
            imageops::overlay(&mut image, &glyph, x.into(), 0);
            x += GLYPH_WIDTH + 2;
        }

        assert_eq!(recognize_text("player1supply", &image), "42/58");
    }

    #[test]
    fn rejects_non_five_glyph_timestamp() {
        let glyphs = ['1', '2', '3', '4'];
        let width = GLYPH_WIDTH as usize * glyphs.len() + (glyphs.len() - 1) * 2;
        let mut image = GrayImage::new(width as u32, GLYPH_HEIGHT);
        let mut x = 0u32;
        for ch in glyphs {
            let glyph = render_char(RoiGroup::Timestamp, ch);
            imageops::overlay(&mut image, &glyph, x.into(), 0);
            x += GLYPH_WIDTH + 2;
        }

        assert_eq!(recognize_text("timestamp", &image), "");
    }

    #[test]
    fn rejects_supply_with_invalid_length() {
        let glyphs = ['4', '/'];
        let width = GLYPH_WIDTH as usize * glyphs.len() + (glyphs.len() - 1) * 2;
        let mut image = GrayImage::new(width as u32, GLYPH_HEIGHT);
        let mut x = 0u32;
        for ch in glyphs {
            let glyph = render_char(RoiGroup::Supply, ch);
            imageops::overlay(&mut image, &glyph, x.into(), 0);
            x += GLYPH_WIDTH + 2;
        }

        assert_eq!(recognize_text("player1supply", &image), "");
    }

    #[test]
    fn loads_real_templates_from_disk() {
        let temp_root = std::env::temp_dir().join(format!(
            "bwl-meta-test-{}-{}",
            std::process::id(),
            "real-templates"
        ));
        let glyph_dir = temp_root.join("supply").join("4");
        fs::create_dir_all(&glyph_dir).unwrap();
        let glyph = render_template(&["...##", "..###", ".####", "##.##", "#####", "...##", "...##"]);
        glyph.save(glyph_dir.join("sample.png")).unwrap();

        let loaded = load_templates_from_dir(&temp_root, RoiGroup::Supply).unwrap();
        let four_templates = loaded
            .iter()
            .find(|(glyph, _)| *glyph == '4')
            .map(|(_, images)| images.len())
            .unwrap_or(0);

        assert_eq!(four_templates, 1);

        let _ = fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn segments_multiple_glyphs() {
        let glyphs = ['4', '2', '/', '5'];
        let width = GLYPH_WIDTH as usize * glyphs.len() + (glyphs.len() - 1) * 3;
        let mut image = GrayImage::new(width as u32, GLYPH_HEIGHT);
        let mut x = 0u32;
        for ch in glyphs {
            let glyph = render_char(RoiGroup::Supply, ch);
            imageops::overlay(&mut image, &glyph, x.into(), 0);
            x += GLYPH_WIDTH + 3;
        }

        let segmented = debug_segment_glyphs(&image);

        assert_eq!(segmented.len(), 4);
    }
}

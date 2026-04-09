use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow};
use bwl_meta::{
    models::{Config, FrameData, GoldSetEntry},
    recognizer::{debug_score_glyphs, debug_segment_glyphs},
    dump_frame_debug, parse_frame, parse_segments_with_progress,
};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Extract,
    Eval {
        #[arg(long, default_value = "goldset.jsonl")]
        goldset: PathBuf,
    },
    DebugFrame {
        #[arg(long)]
        time: String,
        #[arg(long)]
        video: Option<PathBuf>,
        #[arg(long, default_value = "debug-rois")]
        out_dir: PathBuf,
    },
    SliceGlyphs {
        #[arg(long)]
        image: PathBuf,
        #[arg(long, default_value = "debug-glyphs")]
        out_dir: PathBuf,
    },
    ScoreGlyphs {
        #[arg(long)]
        roi: String,
        #[arg(long)]
        image: PathBuf,
        #[arg(long, default_value_t = 5)]
        top: usize,
    },
}

#[derive(Default)]
struct FieldStats {
    correct: usize,
    total: usize,
}

impl FieldStats {
    fn record(&mut self, matched: bool) {
        self.total += 1;
        if matched {
            self.correct += 1;
        }
    }

    fn percent(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.correct as f64 / self.total as f64) * 100.0
    }
}

#[derive(Default)]
struct EvalStats {
    timestamp: FieldStats,
    player1supply: FieldStats,
    player2supply: FieldStats,
    frame_matches: usize,
    frame_total: usize,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Commands::Extract) {
        Commands::Extract => run_extract(),
        Commands::Eval { goldset } => run_eval(&goldset),
        Commands::DebugFrame {
            time,
            video,
            out_dir,
        } => run_debug_frame(&time, video.as_deref(), &out_dir),
        Commands::SliceGlyphs { image, out_dir } => run_slice_glyphs(&image, &out_dir),
        Commands::ScoreGlyphs { roi, image, top } => run_score_glyphs(&roi, &image, top),
    }
}

fn run_extract() -> Result<()> {
    let conf = read_config()?;

    ensure_video(&conf)?;

    let total_frames: usize = conf
        .matches
        .iter()
        .map(|segment| {
            let start = parse_frame_time(&segment.start)?;
            let end = parse_frame_time(&segment.end)?;
            Ok(end.saturating_sub(start))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .sum();
    let progress = make_progress_bar(total_frames as u64, "Extracting BW HUD");
    let data = parse_segments_with_progress(&conf, |_| {
        progress.inc(1);
    })?;
    progress.finish_with_message("Extracting BW HUD done");

    let json_file_name = format!("{} {} {}.json", conf.org, conf.org_season, conf.org_xtra);
    fs::write(&json_file_name, serde_json::to_string_pretty(&data)?)?;
    println!("JSON :: {}", json_file_name);

    Ok(())
}

fn run_eval(goldset_path: &Path) -> Result<()> {
    let content = fs::read_to_string(goldset_path)
        .with_context(|| format!("reading {}", goldset_path.display()))?;
    let gold_entries = parse_goldset(&content)?;

    if gold_entries.is_empty() {
        return Err(anyhow!("gold set is empty"));
    }

    let mut stats = EvalStats::default();
    let mut failures = Vec::new();
    let progress = make_progress_bar(gold_entries.len() as u64, "Evaluating gold set");

    for entry in &gold_entries {
        let second = parse_frame_time(&entry.frame_time)
            .with_context(|| format!("parsing frame_time {}", entry.frame_time))?;
        let predicted = parse_frame(&entry.video, second)
            .with_context(|| format!("extracting {} at {}", entry.video, entry.frame_time))?;
        let predicted =
            predicted.ok_or_else(|| anyhow!("no frame decoded for {} at {}", entry.video, entry.frame_time))?;

        stats.frame_total += 1;
        let frame_match = compare_entry(entry, &predicted, &mut stats, &mut failures);
        if frame_match {
            stats.frame_matches += 1;
        }
        progress.inc(1);
    }

    progress.finish_with_message("Evaluating gold set done");
    print_eval_report(goldset_path, &stats, &failures);
    Ok(())
}

fn run_debug_frame(time: &str, video: Option<&Path>, out_dir: &Path) -> Result<()> {
    let second = parse_frame_time(time)?;
    let video_path = match video {
        Some(path) => path.to_path_buf(),
        None => read_config()?.video_name.into(),
    };

    let frame = dump_frame_debug(video_path.to_string_lossy().as_ref(), second, out_dir)
        .with_context(|| format!("debugging {} at {}", video_path.display(), time))?;
    let frame =
        frame.ok_or_else(|| anyhow!("no frame decoded for {} at {}", video_path.display(), time))?;

    println!("Debug frame :: {} {}", video_path.display(), time);
    println!("Output dir :: {}", out_dir.display());
    println!("OCR :: {}", serde_json::to_string_pretty(&frame)?);
    Ok(())
}

fn run_slice_glyphs(image_path: &Path, out_dir: &Path) -> Result<()> {
    let image = image::open(image_path)
        .with_context(|| format!("opening {}", image_path.display()))?
        .to_luma8();
    let glyphs = debug_segment_glyphs(&image);
    fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;

    for (idx, glyph) in glyphs.iter().enumerate() {
        let out_path = out_dir.join(format!("{idx:02}.png"));
        glyph.save(&out_path)
            .with_context(|| format!("writing {}", out_path.display()))?;
    }

    println!("Input :: {}", image_path.display());
    println!("Output dir :: {}", out_dir.display());
    println!("Glyphs :: {}", glyphs.len());
    Ok(())
}

fn run_score_glyphs(roi: &str, image_path: &Path, top: usize) -> Result<()> {
    let image = image::open(image_path)
        .with_context(|| format!("opening {}", image_path.display()))?
        .to_luma8();
    let scored = debug_score_glyphs(roi, &image, top);

    println!("ROI :: {roi}");
    println!("Input :: {}", image_path.display());
    println!("Glyphs :: {}", scored.len());
    for (idx, glyph) in scored.iter().enumerate() {
        let summary = glyph
            .scores
            .iter()
            .map(|entry| format!("{} {:.3}", entry.ch, entry.score))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{idx:02} :: accepted={} best={} :: {}",
            glyph.accepted,
            glyph.best.unwrap_or('?'),
            summary
        );
    }
    Ok(())
}

fn read_config() -> Result<Config> {
    let config_text = fs::read_to_string("config.toml").context("reading config.toml")?;
    toml::from_str(&config_text).context("parsing config.toml")
}

fn ensure_video(conf: &Config) -> Result<()> {
    if Path::new(&conf.video_name).exists() {
        return Ok(());
    }

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
        return Err(anyhow!("yt-dlp"));
    }

    Ok(())
}

fn parse_goldset(content: &str) -> Result<Vec<GoldSetEntry>> {
    content
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            Some(
                serde_json::from_str::<GoldSetEntry>(trimmed)
                    .with_context(|| format!("parsing gold set line {}", idx + 1)),
            )
        })
        .collect()
}

fn parse_frame_time(value: &str) -> Result<usize> {
    let parts: Vec<_> = value.split(':').collect();
    if parts.len() != 3 {
        return Err(anyhow!("Invalid timestamp: {}", value));
    }
    let h: usize = parts[0].parse()?;
    let m: usize = parts[1].parse()?;
    let s: usize = parts[2].parse()?;
    Ok(h * 3600 + m * 60 + s)
}

fn compare_entry(
    expected: &GoldSetEntry,
    predicted: &FrameData,
    stats: &mut EvalStats,
    failures: &mut Vec<String>,
) -> bool {
    let timestamp_match = expected.timestamp == predicted.timestamp;
    stats.timestamp.record(timestamp_match);

    let player1_match = expected.player1supply == predicted.player1supply;
    stats.player1supply.record(player1_match);

    let player2_match = expected.player2supply == predicted.player2supply;
    stats.player2supply.record(player2_match);

    if timestamp_match && player1_match && player2_match {
        return true;
    }

    failures.push(format_failure(expected, predicted));
    false
}

fn format_failure(expected: &GoldSetEntry, predicted: &FrameData) -> String {
    format!(
        "{} {} :: timestamp expected={} predicted={} | player1supply expected={} predicted={} | player2supply expected={} predicted={}",
        expected.video,
        expected.frame_time,
        expected.timestamp,
        predicted.timestamp,
        expected.player1supply.as_deref().unwrap_or("<none>"),
        predicted.player1supply.as_deref().unwrap_or("<none>"),
        expected.player2supply.as_deref().unwrap_or("<none>"),
        predicted.player2supply.as_deref().unwrap_or("<none>"),
    )
}

fn print_eval_report(goldset_path: &Path, stats: &EvalStats, failures: &[String]) {
    println!("Gold set :: {}", goldset_path.display());
    println!(
        "Frames :: {}/{} ({:.1}%)",
        stats.frame_matches,
        stats.frame_total,
        percent(stats.frame_matches, stats.frame_total)
    );
    println!(
        "timestamp :: {}/{} ({:.1}%)",
        stats.timestamp.correct,
        stats.timestamp.total,
        stats.timestamp.percent()
    );
    println!(
        "player1supply :: {}/{} ({:.1}%)",
        stats.player1supply.correct,
        stats.player1supply.total,
        stats.player1supply.percent()
    );
    println!(
        "player2supply :: {}/{} ({:.1}%)",
        stats.player2supply.correct,
        stats.player2supply.total,
        stats.player2supply.percent()
    );

    if failures.is_empty() {
        println!("Failures :: none");
        return;
    }

    println!("Failures :: {}", failures.len());
    for failure in failures {
        println!("{failure}");
    }
}

fn percent(correct: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (correct as f64 / total as f64) * 100.0
}

fn make_progress_bar(len: u64, message: &str) -> ProgressBar {
    let progress = ProgressBar::new(len);
    let style = ProgressStyle::with_template(
        "{msg} [{elapsed_precise}] [{wide_bar}] {pos}/{len} ({percent}%)",
    )
    .expect("valid progress template")
    .progress_chars("=> ");
    progress.set_style(style);
    progress.set_message(message.to_string());
    progress
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_goldset_jsonl() {
        let content = r#"
{"video":"sample.mp4","frame_time":"00:10:03","timestamp":"10:03","player1supply":"42/58","player2supply":"38/50"}
{"video":"sample.mp4","frame_time":"00:10:04","timestamp":"10:04","player1supply":"42/58","player2supply":"39/50"}
"#;

        let entries = parse_goldset(content).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].frame_time, "00:10:03");
        assert_eq!(entries[1].player2supply.as_deref(), Some("39/50"));
    }

    #[test]
    fn compares_expected_and_predicted_frames() {
        let expected = GoldSetEntry {
            video: "sample.mp4".to_string(),
            frame_time: "00:10:03".to_string(),
            timestamp: "10:03".to_string(),
            player1supply: Some("42/58".to_string()),
            player2supply: Some("38/50".to_string()),
        };
        let predicted = FrameData {
            timestamp: "10:03".to_string(),
            player1supply: Some("42/58".to_string()),
            player2supply: Some("37/50".to_string()),
        };

        let mut stats = EvalStats::default();
        let mut failures = Vec::new();
        let matched = compare_entry(&expected, &predicted, &mut stats, &mut failures);

        assert!(!matched);
        assert_eq!(stats.timestamp.correct, 1);
        assert_eq!(stats.player1supply.correct, 1);
        assert_eq!(stats.player2supply.correct, 0);
        assert_eq!(failures.len(), 1);
    }
}

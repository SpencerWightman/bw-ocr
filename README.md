# BWL Meta Server

`bwl-meta-server` is the Brood War-specific metadata extractor built on top of `pixelpipe`.

`pixelpipe` owns the generic video/frame layer:
- FFmpeg-backed frame decoding
- ROI cropping
- grayscale / resize / threshold preprocessing

`bwl-meta-server` owns the BW-specific layer:
- hardcoded BW HUD ROIs
- BW text recognition
- frame-level metadata extraction
- later clip-level aggregation and evaluation

The current extractor handles:
- `timestamp`
- `player1supply`
- `player2supply`

It no longer uses Tesseract. It now uses a built-in BW-specific recognizer for HUD digits plus `:` and `/`.

## Repo Status

Implemented now:
- segment decoding through `pixelpipe`
- ROI preprocessing through `pixelpipe`
- BW-specific HUD recognizer in `src/recognizer.rs`
- runtime `config.toml` loading
- gold-set evaluation command

Not implemented yet:
- resources extraction
- unit/base extraction
- clip-level aggregation

## Prerequisites

- Rust toolchain
- `ffmpeg` on `PATH`
- `yt-dlp` on `PATH` if you want automatic video download
- a runtime `config.toml` in the repo root

## Build And Test

```bash
cd /Users/spencer/Documents/zip/bwl-meta-server
cargo check
cargo test
```

## Runtime Config

Create `/Users/spencer/Documents/zip/bwl-meta-server/config.toml`.

Example:

```toml
yt_url = "https://www.youtube.com/watch?v=..."
video_name = "sample.mp4"
date = "2026-04-07"
org = "BWL"
org_season = 1
org_xtra = "sample"

[[matches]]
player1 = "PlayerA"
race1 = "P"
player2 = "PlayerB"
race2 = "Z"
winner = "PlayerA"
start = "00:10:00"
end = "00:12:00"
```

## How To Use It

### 1. Run extraction

```bash
cd /Users/spencer/Documents/zip/bwl-meta-server
cargo run
```

The binary will:
- load `config.toml`
- download `video_name` with `yt-dlp` if the file does not already exist
- decode each configured segment at `1 fps`
- extract timestamp and supply metadata from BW HUD ROIs
- show extraction progress while frames are being decoded and OCR'd
- write JSON output named like `<org> <org_season> <org_xtra>.json`

### 2. Inspect output

The JSON output contains one entry per configured match segment and one OCR record per extracted frame.

Current frame fields:
- `timestamp`
- `player1supply`
- `player2supply`

### 3. Run gold-set evaluation

Put a JSONL file at `/Users/spencer/Documents/zip/bwl-meta-server/goldset.jsonl`, or pass a custom path.

Example:

```json
{"video":"sample.mp4","frame_time":"00:10:03","timestamp":"10:03","player1supply":"42/58","player2supply":"38/50"}
{"video":"sample.mp4","frame_time":"00:10:04","timestamp":"10:04","player1supply":"42/58","player2supply":"39/50"}
```

Run:

```bash
cd /Users/spencer/Documents/zip/bwl-meta-server
cargo run -- eval
```

Or with a custom path:

```bash
cargo run -- eval --goldset data/goldset.jsonl
```

The evaluator:
- reads each labeled frame from the gold set
- decodes that exact second from the referenced video
- extracts `timestamp`, `player1supply`, and `player2supply`
- shows progress while labeled frames are being evaluated
- prints exact-match accuracy per field and a failure list

### 4. Update ROIs

BW-specific ROIs live in `src/constants.rs`.

If you need to adjust timestamp or supply extraction for another overlay style, change those ROI coordinates there.

### 5. Dump debug ROI crops

To inspect what OCR is seeing for one frame:

```bash
cd /Users/spencer/Documents/zip/bwl-meta-server
cargo run -- debug-frame --time 01:52:10 --video my-bw-game.mp4 --out-dir debug-rois
```

If `--video` is omitted, the command uses `video_name` from `config.toml`.

The command writes:
- `timestamp-crop.png`
- `timestamp-bw.png`
- `player1supply-crop.png`
- `player1supply-bw.png`
- `player2supply-crop.png`
- `player2supply-bw.png`
- `ocr.json`

Use those images to tune ROI coordinates and thresholding.

### 6. Add real glyph templates

The recognizer now loads real BW glyph templates from `/Users/spencer/Documents/zip/bwl-meta-server/templates/glyphs` before falling back to the built-in synthetic templates.

Timestamp and supply use separate template sets, because their fonts can differ.
Timestamp `:` is structural now and does not need OCR templates.

Directory layout:

```text
templates/glyphs/
  timestamp/
    0/
    1/
    2/
    3/
    4/
    5/
    6/
    7/
    8/
    9/
  supply/
    0/
    1/
    2/
    3/
    4/
    5/
    6/
    7/
    8/
    9/
    slash/
```

Drop thresholded glyph PNGs into the matching directory, for example:

```text
templates/glyphs/timestamp/9/frame-015235-1.png
templates/glyphs/supply/4/frame-015503-1.png
templates/glyphs/supply/slash/frame-015503-2.png
```

The loader normalizes each PNG to the recognizer size and uses the best match across all templates for that character. If no real templates exist, the recognizer falls back to the built-in glyph set.

You can override the default template directory with:

```bash
BWL_TEMPLATE_DIR=/some/other/path cargo run
```

### 7. Slice Segmented Glyphs

To inspect how the recognizer is splitting a thresholded ROI into glyphs:

```bash
cargo run -- slice-glyphs --image debug-rois/frame-015235/timestamp-bw.png --out-dir debug-glyphs/timestamp-015235
```

This command writes numbered glyph images like:

```text
debug-glyphs/timestamp-015235/00.png
debug-glyphs/timestamp-015235/01.png
debug-glyphs/timestamp-015235/02.png
```

Run it against:
- `timestamp-bw.png`
- `player1supply-bw.png`
- `player2supply-bw.png`

If the numbered glyph images already look wrong, the problem is segmentation rather than template matching.

### 8. Score Glyph Matches

To inspect the top character matches for each segmented glyph in a thresholded ROI:

```bash
cargo run -- score-glyphs --roi timestamp --image debug-rois/frame-015235/timestamp-bw.png --top 5
```

For supply:

```bash
cargo run -- score-glyphs --roi player2supply --image debug-rois/frame-015235/player2supply-bw.png --top 5
```

This prints, for each segmented glyph, whether the current threshold would accept it and the top-ranked character scores. If the correct glyph is consistently second or third, the scoring/templates are the problem.

## How It Works

### `pixelpipe`
`pixelpipe` is used for:
- decoding frames from video segments
- cropping ROIs from frames
- resizing and thresholding OCR inputs

### `bwl-meta-server`
`bwl-meta-server` is used for:
- BW ROI definitions
- BW recognizer logic
- frame-level metadata extraction
- temporal consistency cleanup across adjacent frames

The recognizer path is:
1. decode a frame from the segment
2. crop the BW HUD ROI with `pixelpipe`
3. grayscale, enlarge, and threshold the crop with `pixelpipe`
4. segment glyphs from the binary image
5. classify glyphs against a built-in BW glyph template set
6. write the extracted text into frame metadata

## Validate The Recognizer On Real BW Frames

The current recognizer passes synthetic unit tests. The next step is validating it on real footage.

Recommended workflow:

1. Create a small `config.toml` with 3-5 short BW segments.
2. Run `cargo run`.
3. Compare JSON output against the actual video for:
   - `timestamp`
   - `player1supply`
   - `player2supply`
4. Record the correct values for a small subset of real frames.

Start with a small gold subset of 20-50 real frames.

Suggested fields to record per labeled frame:

```json
{"video":"sample.mp4","frame_time":"00:10:03","timestamp":"10:03","player1supply":"42/58","player2supply":"38/50"}
```

What to look for:
- colon mistakes in timestamps
- slash mistakes in supply strings
- dropped digits
- overlay-dependent failures
- failures caused by compression or brightness

Important note:
- some `null` values are expected in real BW VODs
- the broadcast may briefly cut away from the in-game HUD
- a sampled frame may catch a HUD digit mid-render, producing a partial glyph
- in those cases, `null` is preferable to forcing an incorrect value

## Expand HUD Extraction To Resources

The next field to add should be resources.

Recommended order:

1. Add new BW ROIs in `src/constants.rs` for:
   - `player1resources`
   - `player2resources`
2. Extend `FrameData` in `src/models.rs` with matching optional fields.
3. Reuse the current preprocessing + recognizer path.
4. Add recognizer tests for resource-style strings.
5. Validate those fields on real labeled frames.

Do not add this logic to `pixelpipe`. Resource extraction is BW-specific and belongs in `bwl-meta-server`.

## Gold-Set Evaluation

A gold set is a small labeled dataset of real BW frames used to measure recognizer quality.

Suggested file format:

```json
{"video":"sample.mp4","frame_time":"00:10:03","timestamp":"10:03","player1supply":"42/58","player2supply":"38/50"}
{"video":"sample.mp4","frame_time":"00:10:04","timestamp":"10:04","player1supply":"42/58","player2supply":"39/50"}
```

Later, when resources are added:

```json
{"video":"sample.mp4","frame_time":"00:10:05","timestamp":"10:05","player1supply":"44/58","player2supply":"41/50","player1resources":"312","player2resources":"188"}
```

Current evaluation loop:

1. Create `goldset.jsonl` with labeled real frames.
2. Run `cargo run -- eval`.
3. Review exact-match accuracy per field.
4. Inspect the printed failure list.
5. Rerun after any ROI, preprocessing, or recognizer change.

## Recommended Next Steps

1. Validate timestamp and supply extraction on real BW frames.
2. Add resources as the next HUD field.
3. Create a gold-set file and a simple evaluator.
4. After HUD fields are stable, move on to unit/base extraction and clip-level aggregation.

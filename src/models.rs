use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Config {
    pub yt_url: String,
    pub video_name: String,
    pub date: String,
    pub org: String,
    pub org_season: u8,
    pub org_xtra: String,
    pub matches: Vec<ConfigMatchSegment>,
}

#[derive(Serialize, Deserialize)]
pub struct ConfigMatchSegment {
    pub player1: String,
    pub race1: String,
    pub player2: String,
    pub race2: String,
    pub winner: String,
    #[serde(skip_serializing)]
    pub start: String,
    #[serde(skip_serializing)]
    pub end: String,
}

#[derive(Serialize)]
pub struct EntryData<'a> {
    #[serde(flatten)]
    pub segment: &'a ConfigMatchSegment,
    pub ocr: Vec<FrameData>,
}

#[derive(Serialize)]
pub struct FrameData {
    pub timestamp: String,
    pub player1supply: Option<String>,
    pub player2supply: Option<String>,
}

pub struct Roi {
    pub name: &'static str,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

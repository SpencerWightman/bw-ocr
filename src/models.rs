use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Config {
    pub yt_url: String,
    pub date: String,
    pub org: String,
    pub org_season: u8,
    pub org_xtra: u8,
    pub matches: Vec<MatchSegment>,
}

#[derive(Deserialize)]
pub struct MatchSegment {
    pub player1: String,
    pub race1: String,
    pub player2: String,
    pub race2: String,
    pub winner: String,
    pub start: String,
    pub end: String,
}

#[derive(Serialize)]
pub struct SupplyData {
    pub player1_supply: String,
    pub player2_supply: String,
}

pub struct Roi {
    pub name: &'static str,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

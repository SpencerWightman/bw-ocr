use anyhow::{Ok, Result};

use crate::{anyhow, models::SupplyData};

pub fn parse_timestamp(tc: &str) -> Result<f64> {
    let parts: Vec<_> = tc.split(':').collect();
    if parts.len() != 3 {
        return Err(anyhow!("Invalid timecode: {}", tc));
    }
    let h: f64 = parts[0].parse()?;
    let m: f64 = parts[1].parse()?;
    let s: f64 = parts[2].parse()?;
    Ok(h * 3600.0 + m * 60.0 + s)
}

pub fn supplies_diff(c_frame: &SupplyData, n_frame: &SupplyData) -> bool {
    let player1_frame1 = normalize_slash(&c_frame.player1_supply);
    let player1_frame2 = normalize_slash(&n_frame.player1_supply);
    let player2_frame1 = normalize_slash(&c_frame.player2_supply);
    let player2_frame2 = normalize_slash(&n_frame.player2_supply);
    if player1_frame1 != player1_frame2 || player2_frame1 != player2_frame2 {
        return true;
    }
    false
}

fn normalize_slash(s: &str) -> String {
    s.split('/').map(str::trim).collect::<Vec<&str>>().join("/")
}

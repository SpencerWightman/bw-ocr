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

// Do values match next or previous
pub fn find_consistent(
    p_frame: &SupplyData,
    c_frame: &SupplyData,
    n_frame: &SupplyData,
) -> Result<(i32, i32)> {
    let c1 = normalize_slash(get_str(&c_frame.player1_supply))?;
    let n1 = normalize_slash(get_str(&n_frame.player1_supply))?;
    let p1 = normalize_slash(get_str(&p_frame.player1_supply))?;

    let c2 = normalize_slash(get_str(&c_frame.player2_supply))?;
    let n2 = normalize_slash(get_str(&n_frame.player2_supply))?;
    let p2 = normalize_slash(get_str(&p_frame.player2_supply))?;

    println!("p: {:?}, c: {:?}, n: {:?}", (p2), (c2), (n2));

    let mut consistent = (1, 1);

    if c1 != n1 && c1 != p1 {
        consistent.0 = 0;
    }

    if c2 != n2 && c2 != p2 {
        consistent.1 = 0;
    }

    Ok(consistent)
}

fn get_str(supply_opt: &Option<String>) -> String {
    supply_opt
        .clone()
        .unwrap_or_else(|| "1000/1000".to_string())
}

fn normalize_slash(txt: String) -> Result<String> {
    Ok(txt
        .split('/')
        .map(str::trim)
        .collect::<Vec<&str>>()
        .join("/"))
}

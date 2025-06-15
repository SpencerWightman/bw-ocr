use crate::models::Roi;
pub const THRESH: u8 = 130;
pub const CONFIG_TOML: &str = include_str!("../config.toml");
pub const ROIS: [Roi; 3] = [
    Roi {
        name: "timestamp",
        x: 124,
        y: 738,
        width: 73,
        height: 25,
    },
    Roi {
        name: "player1supply",
        x: 1708,
        y: 16,
        width: 117,
        height: 29,
    },
    Roi {
        name: "player2supply",
        x: 1708,
        y: 46,
        width: 117,
        height: 29,
    },
];

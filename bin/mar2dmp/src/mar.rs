use anyhow::{bail, Result};
use itertools::Itertools;

use gbalz77::{compress, CompressionStrategy};

pub fn process_mar(width: usize, height: usize, raw: &[u8]) -> Result<Vec<u8>> {
    let mut result: Vec<u8> = Vec::new();

    if width * height * 2 != raw.len() {
        bail!(
            "the provided size of the map does not match the size of the file"
        );
    }

    result.push(width as u8);
    result.push(height as u8);

    for (&x, &y) in raw.iter().tuples::<(_, _)>() {
        let xy: u16 = (u16::from_le_bytes([x, y])) >> 3;
        result.extend(xy.to_le_bytes());
    }

    Ok(compress(
        &result[..],
        CompressionStrategy::CheckMostRecentOnly,
    ))
}

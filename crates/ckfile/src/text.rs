/// Decodes Windows-1252 bytes (the encoding Virtools 2.1 used on Western
/// European systems) into a `String`, stopping at the first NUL.
pub fn decode_cp1252(bytes: &[u8]) -> String {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}',
        '\u{017D}', '\u{008F}', '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
        '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
    ];
    bytes
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| match b {
            0x80..=0x9F => HIGH[(b - 0x80) as usize],
            _ => char::from(b),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::decode_cp1252;

    #[test]
    fn decodes_latin1_and_cp1252_ranges() {
        assert_eq!(decode_cp1252(b"\xDCberblendung\0junk"), "Überblendung");
        assert_eq!(decode_cp1252(b"\x80 \x99"), "€ ™");
        assert_eq!(decode_cp1252(b""), "");
    }
}

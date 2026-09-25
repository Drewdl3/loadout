//! Human durations for settings like `sync_interval = "1h"`.

use std::time::Duration;

use crate::ModelError;

/// Parses `<n><group>` where group is `s`, `m`, `h` or `d` (e.g. `90s`,
/// `30m`, `1h`, `7d`). Zero is rejected.
pub fn parse_duration(s: &str) -> Result<Duration, ModelError> {
    let bad = || ModelError::Duration(s.to_owned());
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).ok_or_else(bad)?;
    let (num, group) = s.split_at(split);
    let n: u64 = num.parse().map_err(|_| bad())?;
    let secs = match group.trim() {
        "s" => n,
        "m" => n.checked_mul(60).ok_or_else(bad)?,
        "h" => n.checked_mul(3600).ok_or_else(bad)?,
        "d" => n.checked_mul(86_400).ok_or_else(bad)?,
        _ => return Err(bad()),
    };
    if secs == 0 {
        return Err(bad());
    }
    Ok(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_groups() {
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_duration(" 1h ").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("7d").unwrap(), Duration::from_secs(604_800));
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            "",
            "h",
            "1",
            "1w",
            "0h",
            "-1h",
            "1.5h",
            "99999999999999999999d",
        ] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
    }
}

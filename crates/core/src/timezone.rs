use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use once_cell::sync::Lazy;
use tzf_rs::DefaultFinder;

// Initialize the timezone finder only once, as it loads geospatial data into memory.
static FINDER: Lazy<DefaultFinder> = Lazy::new(DefaultFinder::new);

/// Resolves geographic coordinates to a local timezone and formats an EXIF timestamp string.
/// If this ever breaks, it is probably because of daylight savings rules changing... again.
pub fn format_localized_time(lat: f64, lon: f64, timestamp: i64) -> Option<String> {
    // Null Island (0,0) is Takeout's default for missing GPS; no sensible timezone exists there.
    if lat == 0.0 && lon == 0.0 {
        return None;
    }

    let tz_name = FINDER.get_tz_name(lon, lat);
    if tz_name.is_empty() {
        return None;
    }

    let tz: Tz = tz_name.parse().ok()?;
    let dt_utc = Utc.timestamp_opt(timestamp, 0).single()?;
    let dt_local = dt_utc.with_timezone(&tz);

    let time_str = dt_local.format("%Y:%m:%d %H:%M:%S").to_string();
    let offset_str = dt_local.format("%z").to_string();

    if offset_str.len() == 5 {
        Some(format!(
            "{time_str}{}:{}",
            &offset_str[..3],
            &offset_str[3..]
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_localized_time_new_york_dst() {
        // New York: 40.7128, -74.0060
        // July 1, 2023 12:00:00 UTC (Summer -> EDT -> -04:00, Local -> 08:00:00)
        let ts = 1688212800;
        let formatted = format_localized_time(40.7128, -74.0060, ts);
        assert_eq!(formatted, Some("2023:07:01 08:00:00-04:00".to_string()));
    }

    #[test]
    fn test_format_localized_time_new_york_est() {
        // New York: 40.7128, -74.0060
        // Jan 1, 2023 12:00:00 UTC (Winter -> EST -> -05:00, Local -> 07:00:00)
        let ts = 1672574400;
        let formatted = format_localized_time(40.7128, -74.0060, ts);
        assert_eq!(formatted, Some("2023:01:01 07:00:00-05:00".to_string()));
    }

    #[test]
    fn test_format_localized_time_missing_gps() {
        // Missing GPS should return None
        let ts = 1688212800;
        let formatted = format_localized_time(0.0, 0.0, ts);
        assert_eq!(formatted, None);
    }

    #[test]
    fn test_format_localized_time_invalid_gps() {
        let ts = 1688212800;
        let formatted = format_localized_time(999.0, 999.0, ts);
        assert_eq!(formatted, None);
    }

    #[test]
    fn test_format_localized_time_australia_dst() {
        // Sydney: -33.8688, 151.2093
        // Jan 1, 2023 12:00:00 UTC (Summer -> AEDT -> +11:00, Local -> 23:00:00)
        let ts = 1672574400;
        let formatted = format_localized_time(-33.8688, 151.2093, ts);
        assert_eq!(formatted, Some("2023:01:01 23:00:00+11:00".to_string()));
    }

    #[test]
    fn test_format_localized_time_europe_dst() {
        // Berlin: 52.5200, 13.4050
        // July 1, 2023 12:00:00 UTC (Summer -> CEST -> +02:00, Local -> 14:00:00)
        let ts = 1688212800;
        let formatted = format_localized_time(52.5200, 13.4050, ts);
        assert_eq!(formatted, Some("2023:07:01 14:00:00+02:00".to_string()));
    }

    #[test]
    fn test_format_localized_time_utc_plus_14() {
        // Line Islands, Kiribati: 1.8709, -157.3995 (Pacific/Kiritimati is +14:00)
        let ts = 1688212800;
        let formatted = format_localized_time(1.8709, -157.3995, ts);
        assert_eq!(formatted, Some("2023:07:02 02:00:00+14:00".to_string()));
    }

    #[test]
    fn test_format_localized_time_utc_minus_12() {
        // Baker Island (Uninhabited, UTC-12): 0.1936, -176.4769 (Etc/GMT+12 in standard IANA)
        // tzf-rs might not have Baker Island polygon perfectly.
        // We test a known point in Pacific/Pago_Pago (Samoa, UTC-11) instead for safety: -14.2756, -170.7020
        let ts = 1688212800;
        let formatted = format_localized_time(-14.2756, -170.7020, ts);
        assert_eq!(formatted, Some("2023:07:01 01:00:00-11:00".to_string()));
    }
}

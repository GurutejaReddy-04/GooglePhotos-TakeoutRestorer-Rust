use crate::error::AppError;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct GpsCoordinate {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Option<f64>,
}

impl GpsCoordinate {
    pub fn is_null_island(&self) -> bool {
        self.latitude.abs() < f64::EPSILON && self.longitude.abs() < f64::EPSILON
    }

    pub fn is_real(&self) -> bool {
        !self.is_null_island()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub taken_timestamp: i64,
    pub gps: Option<GpsCoordinate>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TakeoutJson {
    title: Option<String>,
    description: Option<String>,
    photo_taken_time: Option<TimestampData>,
    geo_data_exif: Option<GeoData>,
    geo_data: Option<GeoData>,
}

#[derive(Deserialize)]
struct TimestampData {
    timestamp: String,
}

#[derive(Deserialize)]
struct GeoData {
    latitude: f64,
    longitude: f64,
    altitude: f64,
}

pub fn parse(bytes: &[u8]) -> Result<ParsedMetadata, AppError> {
    if bytes.len() > 10_000_000 {
        return Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "JSON file too large",
        )));
    }

    let raw: TakeoutJson = serde_json::from_slice(bytes)?;

    let timestamp_str = raw.photo_taken_time.ok_or(AppError::NoTimestamp)?.timestamp;
    let taken_timestamp = timestamp_str
        .parse::<i64>()
        .map_err(|_| AppError::NoTimestamp)?;

    if taken_timestamp == 0 {
        return Err(AppError::NoTimestamp);
    }

    let geo = raw.geo_data_exif.or(raw.geo_data);
    let gps = geo.map(|g| GpsCoordinate {
        latitude: g.latitude,
        longitude: g.longitude,
        altitude: Some(g.altitude),
    });

    Ok(ParsedMetadata {
        title: raw.title.filter(|s| !s.is_empty()),
        description: raw.description.filter(|s| !s.is_empty()),
        taken_timestamp,
        gps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_json() {
        let json = r#"{
            "title": "photo.jpg",
            "description": "A nice photo",
            "photoTakenTime": {
                "timestamp": "1610000000",
                "formatted": "Jan 7, 2021, 6:13:20 AM UTC"
            },
            "geoData": {
                "latitude": 40.7128,
                "longitude": -74.0060,
                "altitude": 10.0,
                "latitudeSpan": 0.0,
                "longitudeSpan": 0.0
            },
            "geoDataExif": {
                "latitude": 40.7130,
                "longitude": -74.0065,
                "altitude": 12.0,
                "latitudeSpan": 0.0,
                "longitudeSpan": 0.0
            }
        }"#;

        let parsed = parse(json.as_bytes()).unwrap();
        assert_eq!(parsed.taken_timestamp, 1610000000);
        assert_eq!(parsed.title, Some("photo.jpg".to_string()));

        let gps = parsed.gps.unwrap();
        assert_eq!(gps.latitude, 40.7130); // Prefers geoDataExif
    }

    #[test]
    fn test_missing_timestamp() {
        let json = r#"{
            "title": "photo.jpg"
        }"#;

        let err = parse(json.as_bytes()).unwrap_err();
        assert!(matches!(err, AppError::NoTimestamp));
    }
}

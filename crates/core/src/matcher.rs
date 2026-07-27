use crate::config::Config;
use crate::state_db::{FileStatus, JsonEntry, MatchResult, MediaFile};
use once_cell::sync::Lazy;
use rapidfuzz::distance::levenshtein;
use rayon::prelude::*;
use regex::Regex;
use std::collections::HashMap;
use tracing::{debug, warn};

static SUFFIX_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(.+?)\s*\((\d+)\)$").unwrap());

/// Result of a matching attempt against the JSON pool.
#[derive(Debug)]
struct MatchCandidate<'a> {
    pub json: &'a JsonEntry,
    pub tier: u8,
    pub confidence: u8,
}

pub struct Matcher<'a> {
    json_map: HashMap<String, Vec<&'a JsonEntry>>,
    config: &'a Config,
}

impl<'a> Matcher<'a> {
    pub fn new(json_pool: &'a [JsonEntry], config: &'a Config) -> Self {
        let mut json_map: HashMap<String, Vec<&'a JsonEntry>> =
            HashMap::with_capacity(json_pool.len());
        for entry in json_pool {
            json_map.entry(entry.path.folder()).or_default().push(entry);
        }

        Self { json_map, config }
    }

    pub fn match_batch(&self, media_batch: &[MediaFile]) -> Vec<MatchResult> {
        media_batch
            .par_iter()
            .map(|media| {
                let candidate = self.find_best_match(media);

                let (json_path, match_confidence, match_tier, status) = match candidate {
                    Some(c) => {
                        let status = if c.confidence == 100 {
                            FileStatus::Matched
                        } else {
                            FileStatus::MatchedLowConfidence
                        };
                        (
                            Some(c.json.path.clone()),
                            Some(c.confidence),
                            Some(c.tier),
                            status,
                        )
                    }
                    None => (None, None, None, FileStatus::Unmatched),
                };

                MatchResult {
                    id: media.id,
                    json_path,
                    match_confidence,
                    match_tier,
                    status,
                }
            })
            .collect()
    }

    fn find_best_match(&self, media: &MediaFile) -> Option<MatchCandidate<'a>> {
        let folder = media.path.folder();
        let candidates = self.json_map.get(&folder)?;

        // Tier 1: Exact Match (e.g. image.jpg -> image.jpg.json)
        let exact_target = format!("{}.json", media.filename);
        if let Some(c) = candidates.iter().find(|j| j.filename == exact_target) {
            return Some(MatchCandidate {
                json: c,
                tier: 1,
                confidence: 100,
            });
        }

        // Tier 2: Stripped Extension Match (e.g. image.jpg -> image.json)
        let stem = media
            .filename
            .strip_suffix(&media.extension)
            .unwrap_or(&media.filename);
        let stripped_target = format!("{}.json", stem);
        if let Some(c) = candidates.iter().find(|j| j.filename == stripped_target) {
            return Some(MatchCandidate {
                json: c,
                tier: 2,
                confidence: 100,
            });
        }

        // Tier 3: Apple Edited Match (e.g. image.jpg -> image-edited.jpg.json or image-edited.json)
        let edited_variants = [
            format!("{}-edited{}.json", stem, media.extension),
            format!("{}-edited.json", stem),
            format!("{}-EDITED{}.json", stem, media.extension),
            format!("{}-EDITED.json", stem),
        ];
        for target in &edited_variants {
            if let Some(c) = candidates.iter().find(|j| j.filename == *target) {
                return Some(MatchCandidate {
                    json: c,
                    tier: 3,
                    confidence: 100,
                });
            }
        }

        // Tier 3.5: (N) Suffix Movement
        if let Some(captures) = SUFFIX_REGEX.captures(stem) {
            let clean_base = captures.get(1).unwrap().as_str();
            let n = captures.get(2).unwrap().as_str();

            let suffix_variants = [
                format!("{}{}({}).json", clean_base, media.extension, n),
                format!("{}({}).json", clean_base, n),
                format!(
                    "{}{}({}).supplemental-metadata.json",
                    clean_base, media.extension, n
                ),
                format!("{}({}).supplemental-metadata.json", clean_base, n),
            ];

            for target in &suffix_variants {
                if let Some(c) = candidates.iter().find(|j| j.filename == *target) {
                    debug!(
                        "Matched tier 3.5 suffix movement: {} -> {}",
                        media.filename, target
                    );
                    return Some(MatchCandidate {
                        json: c,
                        tier: 3,
                        confidence: 100,
                    });
                }
            }
        }

        // Tier 4: Live Photo Pairing
        let ext_lower = media.extension.to_lowercase();
        if ext_lower == ".mov" || ext_lower == ".mp4" {
            let possible_image_exts = [".heic", ".HEIC", ".jpg", ".JPG", ".jpeg", ".JPEG"];
            for img_ext in &possible_image_exts {
                let live_target = format!("{}{}.json", stem, img_ext);
                if let Some(c) = candidates.iter().find(|j| j.filename == live_target) {
                    return Some(MatchCandidate {
                        json: c,
                        tier: 4,
                        confidence: 100,
                    });
                }
            }
        }

        // Tier 4.5: MP4 Truncation (video.mp4 -> video.mp.json)
        if ext_lower == ".mp4" {
            let mp_target = format!("{}.mp.json", stem);
            if let Some(c) = candidates.iter().find(|j| j.filename == mp_target) {
                return Some(MatchCandidate {
                    json: c,
                    tier: 4,
                    confidence: 100,
                });
            }
        }

        // Tier 5: Progressive Truncation
        let char_boundaries: Vec<usize> = stem
            .char_indices()
            .map(|(idx, _)| idx)
            .chain(std::iter::once(stem.len()))
            .collect();
        let total_chars = char_boundaries.len() - 1;

        if total_chars > self.config.matching.min_truncation_length {
            for &byte_offset in char_boundaries[self.config.matching.min_truncation_length..]
                .iter()
                .rev()
            {
                let truncated = &stem[..byte_offset];
                let variants = [
                    format!("{}.json", truncated),
                    format!("{}{}.json", truncated, media.extension),
                    format!("{}.supplemental-metadata.json", truncated),
                    format!(
                        "{}{}.supplemental-metadata.json",
                        truncated, media.extension
                    ),
                ];

                let matches: Vec<_> = candidates
                    .iter()
                    .filter(|c| variants.contains(&c.filename))
                    .collect();

                if matches.len() == 1 {
                    debug!(
                        "Matched tier 5 progressive truncation: {} -> {}",
                        media.filename, matches[0].filename
                    );
                    return Some(MatchCandidate {
                        json: matches[0],
                        tier: 5,
                        confidence: 90,
                    });
                }
            }
        }

        // Tier 5.5: Supplemental Metadata Naming
        let supp_variants = [
            format!("{}{}.supplemental-metadata.json", stem, media.extension),
            format!("{}.supplemental-metadata.json", stem),
        ];
        for target in &supp_variants {
            if let Some(c) = candidates.iter().find(|j| j.filename == *target) {
                debug!(
                    "Matched tier 5.5 supplemental metadata: {} -> {}",
                    media.filename, target
                );
                return Some(MatchCandidate {
                    json: c,
                    tier: 5,
                    confidence: 100,
                });
            }
        }

        // Tier 6: Levenshtein Fuzzy Match
        if candidates.len() > 5000 {
            warn!("Fuzzy matching skipped for folder {} with {} candidates (> 5000) to prevent O(N^2)", folder, candidates.len());
            return None;
        }

        let mut best_fuzzy: Option<MatchCandidate> = None;
        let media_base_lower = stem.to_lowercase();
        let media_base_len = media_base_lower.chars().count();

        let dynamic_threshold = std::cmp::min(
            self.config.matching.levenshtein_threshold,
            std::cmp::max(1, (media_base_len / 10) as u32),
        );

        let mut min_distance = dynamic_threshold as usize + 1;

        for c in candidates.iter() {
            let json_base = c.filename.to_lowercase();
            // Compare base names without extension if possible
            let mut json_stem = json_base.strip_suffix(".json").unwrap_or(&json_base);
            json_stem = json_stem
                .strip_suffix(".supplemental-metadata")
                .unwrap_or(json_stem);

            let json_stem_len = json_stem.chars().count();

            // Length pre-filter
            if (media_base_len as isize - json_stem_len as isize).abs() > dynamic_threshold as isize
            {
                continue;
            }

            let dist = levenshtein::distance(media_base_lower.chars(), json_stem.chars());
            if dist < min_distance {
                min_distance = dist;
                let conf = (100 - (dist * 10)) as u8; // heuristic
                best_fuzzy = Some(MatchCandidate {
                    json: c,
                    tier: 6,
                    confidence: conf,
                });
            }
        }

        if let Some(fuzzy) = best_fuzzy {
            if min_distance <= dynamic_threshold as usize {
                debug!(
                    "Matched tier 6 fuzzy: {} -> {}",
                    media.filename, fuzzy.json.filename
                );
                return Some(fuzzy);
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::FilePath;
    use std::path::PathBuf;

    fn create_media(id: i64, filename: &str, ext: &str) -> MediaFile {
        MediaFile {
            id,
            path: FilePath::Real {
                base_components: 0,
                abs: PathBuf::from(format!("/test/{}", filename)),
            },
            filename: filename.to_string(),
            extension: ext.to_string(),
            size: 100,
            status: FileStatus::Pending,
            json_path: None,
            match_confidence: None,
            match_tier: None,
            error_message: None,
            has_live_video: false,
        }
    }

    fn create_json(filename: &str) -> JsonEntry {
        JsonEntry {
            id: 0,
            path: FilePath::Real {
                base_components: 0,
                abs: PathBuf::from(format!("/test/{}", filename)),
            },
            filename: filename.to_string(),
        }
    }

    #[test]
    fn test_tier_1_exact() {
        let json_pool = vec![create_json("image.jpg.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "image.jpg", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 1);
        assert_eq!(result.json.filename, "image.jpg.json");
    }

    #[test]
    fn test_tier_2_stripped() {
        let json_pool = vec![create_json("image.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "image.jpg", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 2);
        assert_eq!(result.json.filename, "image.json");
    }

    #[test]
    fn test_tier_3_5_suffix_movement() {
        let json_pool = vec![create_json("photo.jpg(1).json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "photo(1).jpg", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 3);
        assert_eq!(result.json.filename, "photo.jpg(1).json");
    }

    #[test]
    fn test_tier_3_5_suffix_with_space() {
        let json_pool = vec![create_json("photo.jpg(2).json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "photo (2).jpg", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 3);
        assert_eq!(result.json.filename, "photo.jpg(2).json");
    }

    #[test]
    fn test_tier_4_mp4_truncation() {
        let json_pool = vec![create_json("video.mp.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "video.mp4", ".mp4");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 4);
    }

    #[test]
    fn test_tier_5_progressive_truncation() {
        let json_pool = vec![create_json("very_long_file_n.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "very_long_file_name.jpg", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 5);
        assert_eq!(result.json.filename, "very_long_file_n.json");
    }

    #[test]
    fn test_tier_5_5_supplemental_metadata() {
        let json_pool = vec![create_json("image.jpg.supplemental-metadata.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "image.jpg", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 5);
        assert_eq!(result.json.filename, "image.jpg.supplemental-metadata.json");
    }

    #[test]
    fn test_tier_6_fuzzy() {
        let json_pool = vec![create_json("imoge_123.json")];
        let mut config = Config::default();
        config.matching.levenshtein_threshold = 3;
        let matcher = Matcher::new(&json_pool, &config);

        let media = create_media(1, "image_123.jpg", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 6);
        assert_eq!(result.confidence, 90);
    }

    #[test]
    fn test_tier_6_dynamic_threshold() {
        let json_pool = vec![create_json("short1.json")];
        let mut config = Config::default();
        config.matching.levenshtein_threshold = 3;
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "short.jpg", ".jpg");

        // dist from short to short1 is 1. max(1, 5/10) = 1. min(3, 1) = 1.
        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 6);
        assert_eq!(result.json.filename, "short1.json");

        // dist 2 should fail
        let json_pool2 = vec![create_json("short12.json")];
        let matcher2 = Matcher::new(&json_pool2, &config);
        let result2 = matcher2.find_best_match(&media);
        assert!(result2.is_none());
    }

    #[test]
    fn test_tier_6_folder_cap() {
        let mut json_pool = Vec::new();
        for i in 0..5001 {
            json_pool.push(create_json(&format!("image{}.json", i)));
        }
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "image_fuzzy.jpg", ".jpg");

        // Should skip fuzzy matching and return None
        let result = matcher.find_best_match(&media);
        assert!(result.is_none());
    }

    #[test]
    fn test_live_photo_pairing() {
        let json_pool = vec![create_json("IMG_1234.HEIC.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "IMG_1234.MOV", ".MOV");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 4); // Live Photo tier returns 4
        assert_eq!(result.json.filename, "IMG_1234.HEIC.json");
    }

    #[test]
    fn test_46_char_truncation() {
        let long_name = "12345678901234567890123456789012345678901234567890";
        let truncated_json = "1234567890123456789012345678901234567890123456.json";

        let json_pool = vec![create_json(truncated_json)];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, long_name, ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 5);
        assert_eq!(result.json.filename, truncated_json);
    }

    #[test]
    fn test_tier_5_unicode_progressive_truncation() {
        let long_unicode_stem = "照片_相册_旅行_2023_very_long_filename_string";
        let truncated_json = "照片_相册_旅行_2023_very.json";

        let json_pool = vec![create_json(truncated_json)];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, &format!("{}.jpg", long_unicode_stem), ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 5);
        assert_eq!(result.json.filename, truncated_json);
    }

    #[test]
    fn test_tier_5_emoji_progressive_truncation() {
        let long_emoji_stem = "📷_vacation_picture_2023_extra_long";
        let truncated_json = "📷_vacation_picture.json";

        let json_pool = vec![create_json(truncated_json)];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, &format!("{}.jpg", long_emoji_stem), ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 5);
        assert_eq!(result.json.filename, truncated_json);
    }
}

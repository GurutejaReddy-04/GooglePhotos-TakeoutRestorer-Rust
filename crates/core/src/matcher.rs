use crate::config::Config;
use crate::state_db::{FileStatus, JsonEntry, MatchResult, MediaFile};
use once_cell::sync::Lazy;
use rapidfuzz::distance::levenshtein;
use rayon::prelude::*;
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, warn};

static LAST_SUFFIX_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(.*?)\s*\((\d+)\)$").unwrap());
static MULTI_SUFFIX_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(.*?)(?:\s*\(\d+\))+$").unwrap());

static EDITED_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^(.*?)(?:-edited|_edited|\s+-\s+edited|-effects|-effect|\(edited\))(?:\s*\((\d+)\))?$",
    )
    .unwrap()
});

/// Result of a matching attempt against the JSON pool.
#[derive(Debug)]
pub struct MatchCandidate<'a> {
    pub json: &'a JsonEntry,
    pub tier: u8,
    pub confidence: u8,
}

pub struct Matcher<'a> {
    json_map: HashMap<String, HashMap<String, &'a JsonEntry>>,
    json_map_lower: HashMap<String, HashMap<String, &'a JsonEntry>>,
    config: &'a Config,
}

impl<'a> Matcher<'a> {
    pub fn new(json_pool: &'a [JsonEntry], config: &'a Config) -> Self {
        let mut json_map: HashMap<String, HashMap<String, &'a JsonEntry>> = HashMap::new();
        let mut json_map_lower: HashMap<String, HashMap<String, &'a JsonEntry>> = HashMap::new();

        for entry in json_pool {
            let folder = entry.path.folder();
            json_map
                .entry(folder.clone())
                .or_default()
                .insert(entry.filename.clone(), entry);

            json_map_lower
                .entry(folder)
                .or_default()
                .insert(entry.filename.to_lowercase(), entry);
        }

        Self {
            json_map,
            json_map_lower,
            config,
        }
    }

    #[inline]
    fn get_candidate(
        candidates: &HashMap<String, &'a JsonEntry>,
        candidates_lower: Option<&HashMap<String, &'a JsonEntry>>,
        target: &str,
    ) -> Option<&'a JsonEntry> {
        if let Some(&c) = candidates.get(target) {
            return Some(c);
        }
        if let Some(lower_map) = candidates_lower {
            if let Some(&c) = lower_map.get(&target.to_lowercase()) {
                return Some(c);
            }
        }
        None
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

    pub fn find_best_match(&self, media: &MediaFile) -> Option<MatchCandidate<'a>> {
        let folder = media.path.folder();
        let candidates = self.json_map.get(&folder)?;
        let candidates_lower = self.json_map_lower.get(&folder);

        // Robust stem and extension extraction:
        // Use Path::file_stem() so case differences (.JPG vs .jpg) do not corrupt stem extraction.
        let p = Path::new(&media.filename);
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&media.filename);
        let actual_ext = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e))
            .unwrap_or_else(|| media.extension.clone());
        let lower_ext = actual_ext.to_lowercase();

        // Tier 1: Exact sidecar match (e.g. image.jpg -> image.jpg.supplemental-metadata.json or image.jpg.json)
        let exact_targets = [
            format!("{}.supplemental-metadata.json", media.filename),
            format!("{}.json", media.filename),
            format!("{}{}.supplemental-metadata.json", stem, lower_ext),
            format!("{}{}.json", stem, lower_ext),
        ];
        for target in &exact_targets {
            if let Some(c) = Self::get_candidate(candidates, candidates_lower, target) {
                return Some(MatchCandidate {
                    json: c,
                    tier: 1,
                    confidence: 100,
                });
            }
        }

        // Tier 2: Stripped extension match (e.g. image.jpg -> image.supplemental-metadata.json or image.json)
        let stripped_targets = [
            format!("{}.supplemental-metadata.json", stem),
            format!("{}.json", stem),
        ];
        for target in &stripped_targets {
            if let Some(c) = Self::get_candidate(candidates, candidates_lower, target) {
                return Some(MatchCandidate {
                    json: c,
                    tier: 2,
                    confidence: 100,
                });
            }
        }

        // Tier 3: Takeout duplicate index shifting & multi-duplicate resolution
        // Handles:
        // - Single duplicate index: photo(1).jpg, photo(2).heic
        // - Offset index collisions: photo(2).heic where Takeout created photo.heic.supplemental-metadata(1).json
        // - Nested duplicates: photo(1)(1).mov -> photo(1).mov.supplemental-metadata.json or photo.mov.supplemental-metadata(2).json
        // - Unnumbered base fallback: photo(2).heic -> photo.heic.supplemental-metadata.json
        if let Some(last_cap) = LAST_SUFFIX_REGEX.captures(stem) {
            let immediate_base = last_cap.get(1).unwrap().as_str();
            let last_n_str = last_cap.get(2).unwrap().as_str();
            let last_n: u32 = last_n_str.parse().unwrap_or(1);
            let root_base = MULTI_SUFFIX_REGEX
                .captures(stem)
                .map(|c| c.get(1).unwrap().as_str())
                .unwrap_or(immediate_base);

            // Step 3A: If nested duplicates (immediate_base != root_base, e.g. IMG_2072(1)(1) -> IMG_2072(1)),
            // check for dedicated sidecar matching immediate_base directly.
            if immediate_base != root_base {
                let immediate_targets = [
                    format!(
                        "{}{}.supplemental-metadata.json",
                        immediate_base, actual_ext
                    ),
                    format!("{}{}.supplemental-metadata.json", immediate_base, lower_ext),
                    format!("{}.supplemental-metadata.json", immediate_base),
                    format!("{}{}.json", immediate_base, actual_ext),
                    format!("{}{}.json", immediate_base, lower_ext),
                    format!("{}.json", immediate_base),
                ];
                for target in &immediate_targets {
                    if let Some(c) = Self::get_candidate(candidates, candidates_lower, target) {
                        debug!(
                            "Matched tier 3 nested duplicate immediate base: {} -> {}",
                            media.filename, c.filename
                        );
                        return Some(MatchCandidate {
                            json: c,
                            tier: 3,
                            confidence: 100,
                        });
                    }
                }
            }

            // Step 3B: Search candidate indices on root_base.
            // Check in prioritized order:
            // 1. Exact index (last_n, e.g. 2)
            // 2. Off-by-one collision indices (last_n - 1 down to 1, e.g. 1)
            // 3. Common Takeout collision indices (1..=5)
            let mut search_indices = Vec::new();
            search_indices.push(last_n);
            if last_n > 1 {
                for k in (1..last_n).rev() {
                    search_indices.push(k);
                }
            }
            if last_n == 0 {
                search_indices.push(1);
            }
            for k in 1..=5 {
                if !search_indices.contains(&k) {
                    search_indices.push(k);
                }
            }

            for &k in &search_indices {
                let suffix_variants = [
                    // Modern Takeout collision sidecar: base.ext.supplemental-metadata(k).json
                    format!(
                        "{}{}.supplemental-metadata({}).json",
                        root_base, actual_ext, k
                    ),
                    format!(
                        "{}{}.supplemental-metadata({}).json",
                        root_base, lower_ext, k
                    ),
                    format!("{}.supplemental-metadata({}).json", root_base, k),
                    format!(
                        "{}({}){}.supplemental-metadata.json",
                        root_base, k, actual_ext
                    ),
                    format!(
                        "{}({}){}.supplemental-metadata.json",
                        root_base, k, lower_ext
                    ),
                    format!("{}({}).supplemental-metadata.json", root_base, k),
                    // Legacy Takeout collision sidecar formats
                    format!("{}{}({}).json", root_base, actual_ext, k),
                    format!("{}{}({}).json", root_base, lower_ext, k),
                    format!("{}{}.json({})", root_base, actual_ext, k),
                    format!("{}{}.json({})", root_base, lower_ext, k),
                    format!("{}({}).json", root_base, k),
                    format!("{}({}){}.json", root_base, k, actual_ext),
                    format!("{}({}){}.json", root_base, k, lower_ext),
                ];

                for target in &suffix_variants {
                    if let Some(c) = Self::get_candidate(candidates, candidates_lower, target) {
                        debug!(
                            "Matched tier 3 suffix index {}: {} -> {}",
                            k, media.filename, c.filename
                        );
                        return Some(MatchCandidate {
                            json: c,
                            tier: 3,
                            confidence: 100,
                        });
                    }
                }
            }

            // Step 3C: Fallback to unnumbered base photo sidecar if no numbered JSON exists
            // (e.g. Google Takeout only exported photo.ext.supplemental-metadata.json for the series)
            let base_fallback_targets = [
                format!("{}{}.supplemental-metadata.json", root_base, actual_ext),
                format!("{}{}.supplemental-metadata.json", root_base, lower_ext),
                format!("{}.supplemental-metadata.json", root_base),
                format!("{}{}.json", root_base, actual_ext),
                format!("{}{}.json", root_base, lower_ext),
                format!("{}.json", root_base),
            ];
            for target in &base_fallback_targets {
                if let Some(c) = Self::get_candidate(candidates, candidates_lower, target) {
                    debug!(
                        "Matched tier 3 unnumbered base fallback: {} -> {}",
                        media.filename, c.filename
                    );
                    return Some(MatchCandidate {
                        json: c,
                        tier: 3,
                        confidence: 95,
                    });
                }
            }
        }

        // Tier 4: Edited Photo Resolution (e.g. S5001136-edited.JPG -> S5001136.JPG.supplemental-metadata.json)
        // Also supports explicit edited JSONs (S5001136-edited.JPG.json) and Apple Photos exports
        if let Some(captures) = EDITED_REGEX.captures(stem) {
            let base_stem = captures.get(1).unwrap().as_str();
            let duplicate_n = captures.get(2).map(|m| m.as_str());

            let mut edited_targets = vec![
                // 1. Check for dedicated edited sidecar first
                format!("{}.supplemental-metadata.json", media.filename),
                format!("{}.json", media.filename),
                format!("{}.supplemental-metadata.json", stem),
                format!("{}.json", stem),
            ];

            // 2. If duplicate number present on edited photo, check duplicate patterns on base stem
            if let Some(n) = duplicate_n {
                edited_targets.push(format!(
                    "{}{}.supplemental-metadata({}).json",
                    base_stem, actual_ext, n
                ));
                edited_targets.push(format!(
                    "{}{}.supplemental-metadata({}).json",
                    base_stem, lower_ext, n
                ));
                edited_targets.push(format!("{}.supplemental-metadata({}).json", base_stem, n));
                edited_targets.push(format!("{}{}({}).json", base_stem, actual_ext, n));
                edited_targets.push(format!("{}{}({}).json", base_stem, lower_ext, n));
                edited_targets.push(format!("{}({}).json", base_stem, n));
            }

            // 3. Fallback to original photo's metadata (Google Photos often only produces base photo JSON)
            edited_targets.push(format!(
                "{}{}.supplemental-metadata.json",
                base_stem, actual_ext
            ));
            edited_targets.push(format!(
                "{}{}.supplemental-metadata.json",
                base_stem, lower_ext
            ));
            edited_targets.push(format!("{}.supplemental-metadata.json", base_stem));
            edited_targets.push(format!("{}{}.json", base_stem, actual_ext));
            edited_targets.push(format!("{}{}.json", base_stem, lower_ext));
            edited_targets.push(format!("{}.json", base_stem));

            for target in &edited_targets {
                if let Some(c) = Self::get_candidate(candidates, candidates_lower, target) {
                    debug!(
                        "Matched tier 4 edited photo: {} -> {}",
                        media.filename, c.filename
                    );
                    return Some(MatchCandidate {
                        json: c,
                        tier: 4,
                        confidence: 100,
                    });
                }
            }
        }

        // Apple Photos edited exports (media has clean name, sidecar has -edited)
        let apple_edited_variants = [
            format!("{}-edited{}.json", stem, actual_ext),
            format!("{}-edited{}.json", stem, lower_ext),
            format!("{}-edited.json", stem),
            format!("{}-EDITED{}.json", stem, actual_ext),
            format!("{}-EDITED{}.json", stem, lower_ext),
            format!("{}-EDITED.json", stem),
        ];
        for target in &apple_edited_variants {
            if let Some(c) = Self::get_candidate(candidates, candidates_lower, target) {
                return Some(MatchCandidate {
                    json: c,
                    tier: 4,
                    confidence: 100,
                });
            }
        }

        // Tier 5: Live Photo video pairing (pairing .mov/.mp4 with the photo's sidecar)
        if lower_ext == ".mov" || lower_ext == ".mp4" {
            let possible_image_exts = [
                ".heic", ".HEIC", ".jpg", ".JPG", ".jpeg", ".JPEG", ".png", ".PNG",
            ];
            let last_dup_cap = LAST_SUFFIX_REGEX.captures(stem);

            for img_ext in &possible_image_exts {
                let mut live_variants = vec![
                    format!("{}{}.supplemental-metadata.json", stem, img_ext),
                    format!("{}{}.json", stem, img_ext),
                ];

                if let Some(ref cap) = last_dup_cap {
                    let immediate_base = cap.get(1).unwrap().as_str();
                    let last_n_str = cap.get(2).unwrap().as_str();
                    let last_n: u32 = last_n_str.parse().unwrap_or(1);
                    let root_base = MULTI_SUFFIX_REGEX
                        .captures(stem)
                        .map(|c| c.get(1).unwrap().as_str())
                        .unwrap_or(immediate_base);

                    if immediate_base != root_base {
                        live_variants.push(format!(
                            "{}{}.supplemental-metadata.json",
                            immediate_base, img_ext
                        ));
                        live_variants.push(format!("{}{}.json", immediate_base, img_ext));
                    }

                    let mut search_indices = vec![last_n];
                    if last_n > 1 {
                        for k in (1..last_n).rev() {
                            search_indices.push(k);
                        }
                    }
                    if last_n == 0 {
                        search_indices.push(1);
                    }
                    for k in 1..=5 {
                        if !search_indices.contains(&k) {
                            search_indices.push(k);
                        }
                    }

                    for &k in &search_indices {
                        live_variants.push(format!(
                            "{}{}.supplemental-metadata({}).json",
                            root_base, img_ext, k
                        ));
                        live_variants.push(format!("{}{}({}).json", root_base, img_ext, k));
                        live_variants.push(format!("{}{}.json({})", root_base, img_ext, k));
                        live_variants.push(format!(
                            "{}({}){}.supplemental-metadata.json",
                            root_base, k, img_ext
                        ));
                        live_variants.push(format!("{}({}){}.json", root_base, k, img_ext));
                    }

                    // Fallback to unnumbered base image sidecar
                    live_variants.push(format!(
                        "{}{}.supplemental-metadata.json",
                        root_base, img_ext
                    ));
                    live_variants.push(format!("{}{}.json", root_base, img_ext));
                }

                for target in &live_variants {
                    if let Some(c) = Self::get_candidate(candidates, candidates_lower, target) {
                        debug!(
                            "Matched tier 5 live photo pairing: {} -> {}",
                            media.filename, c.filename
                        );
                        return Some(MatchCandidate {
                            json: c,
                            tier: 5,
                            confidence: 100,
                        });
                    }
                }
            }
        }

        // Tier 5.5: Takeout mp4 extension truncation: video.mp4 -> video.mp.json
        if lower_ext == ".mp4" {
            let mp_target = format!("{}.mp.json", stem);
            if let Some(c) = Self::get_candidate(candidates, candidates_lower, &mp_target) {
                return Some(MatchCandidate {
                    json: c,
                    tier: 5,
                    confidence: 100,
                });
            }
        }

        // Tier 6: Progressive Truncation (46-51 char limit)
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
                    format!("{}.supplemental-metadata.json", truncated),
                    format!("{}{}.supplemental-metadata.json", truncated, actual_ext),
                    format!("{}{}.supplemental-metadata.json", truncated, lower_ext),
                    format!("{}.json", truncated),
                    format!("{}{}.json", truncated, actual_ext),
                    format!("{}{}.json", truncated, lower_ext),
                ];

                let mut matches = Vec::new();
                for v in &variants {
                    if let Some(c) = Self::get_candidate(candidates, candidates_lower, v) {
                        matches.push(c);
                    }
                }

                if matches.len() == 1 {
                    debug!(
                        "Matched tier 6 progressive truncation: {} -> {}",
                        media.filename, matches[0].filename
                    );
                    return Some(MatchCandidate {
                        json: matches[0],
                        tier: 6,
                        confidence: 90,
                    });
                }
            }
        }

        // Tier 7: Levenshtein Fuzzy Match
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

        for c in candidates.values() {
            let json_base = c.filename.to_lowercase();
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
                let conf = (100 - (dist * 10)) as u8;
                best_fuzzy = Some(MatchCandidate {
                    json: c,
                    tier: 7,
                    confidence: conf,
                });
            }
        }

        if let Some(fuzzy) = best_fuzzy {
            if min_distance <= dynamic_threshold as usize {
                debug!(
                    "Matched tier 7 fuzzy: {} -> {}",
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
    fn test_tier_1_supplemental_metadata() {
        let json_pool = vec![create_json("image.jpg.supplemental-metadata.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "image.jpg", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 1);
        assert_eq!(result.json.filename, "image.jpg.supplemental-metadata.json");
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
    fn test_tier_3_suffix_movement() {
        let json_pool = vec![create_json("photo.jpg(1).json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "photo(1).jpg", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 3);
        assert_eq!(result.json.filename, "photo.jpg(1).json");
    }

    #[test]
    fn test_takeout_heic_duplicate_supplemental_metadata() {
        // Exactly matches user scenario: IMG_1456(1).HEIC -> IMG_1456.HEIC.supplemental-metadata(1).json
        let json_pool = vec![
            create_json("IMG_1456.HEIC.supplemental-metadata.json"),
            create_json("IMG_1456.HEIC.supplemental-metadata(1).json"),
            create_json("IMG_1456.JPG.supplemental-metadata.json"),
        ];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);

        let media_heic = create_media(1, "IMG_1456.HEIC", ".heic");
        let res_heic = matcher.find_best_match(&media_heic).unwrap();
        assert_eq!(
            res_heic.json.filename,
            "IMG_1456.HEIC.supplemental-metadata.json"
        );

        let media_dup = create_media(2, "IMG_1456(1).HEIC", ".heic");
        let res_dup = matcher.find_best_match(&media_dup).unwrap();
        assert_eq!(
            res_dup.json.filename,
            "IMG_1456.HEIC.supplemental-metadata(1).json"
        );

        let media_jpg = create_media(3, "IMG_1456.JPG", ".jpg");
        let res_jpg = matcher.find_best_match(&media_jpg).unwrap();
        assert_eq!(
            res_jpg.json.filename,
            "IMG_1456.JPG.supplemental-metadata.json"
        );
    }

    #[test]
    fn test_takeout_uppercase_extension_duplicate() {
        let json_pool = vec![create_json("photo.JPG.supplemental-metadata(2).json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "photo(2).JPG", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 3);
        assert_eq!(
            result.json.filename,
            "photo.JPG.supplemental-metadata(2).json"
        );
    }

    #[test]
    fn test_edited_photo_matching_base_metadata() {
        // Matches user scenario: S5001136-edited.JPG -> S5001136.JPG.supplemental-metadata.json
        let json_pool = vec![create_json("S5001136.JPG.supplemental-metadata.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "S5001136-edited.JPG", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 4);
        assert_eq!(
            result.json.filename,
            "S5001136.JPG.supplemental-metadata.json"
        );
    }

    #[test]
    fn test_edited_photo_with_space_and_original_json() {
        let json_pool = vec![create_json("S5001136.JPG.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "S5001136 - edited.JPG", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 4);
        assert_eq!(result.json.filename, "S5001136.JPG.json");
    }

    #[test]
    fn test_edited_photo_with_dedicated_edited_json() {
        let json_pool = vec![create_json(
            "S5001136-edited.JPG.supplemental-metadata.json",
        )];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "S5001136-edited.JPG", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(
            result.json.filename,
            "S5001136-edited.JPG.supplemental-metadata.json"
        );
    }

    #[test]
    fn test_live_photo_mov_with_duplicate_and_supplemental() {
        let json_pool = vec![create_json("IMG_1456.HEIC.supplemental-metadata(1).json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "IMG_1456(1).MOV", ".mov");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 5);
        assert_eq!(
            result.json.filename,
            "IMG_1456.HEIC.supplemental-metadata(1).json"
        );
    }

    #[test]
    fn test_case_insensitive_json_matching() {
        let json_pool = vec![create_json("img_9999.heic.supplemental-metadata.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "IMG_9999.HEIC", ".heic");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(
            result.json.filename,
            "img_9999.heic.supplemental-metadata.json"
        );
    }

    #[test]
    fn test_tier_5_mp4_truncation() {
        let json_pool = vec![create_json("video.mp.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "video.mp4", ".mp4");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 5);
    }

    #[test]
    fn test_tier_6_progressive_truncation() {
        let json_pool = vec![create_json("very_long_file_n.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "very_long_file_name.jpg", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 6);
        assert_eq!(result.json.filename, "very_long_file_n.json");
    }

    #[test]
    fn test_tier_7_fuzzy() {
        let json_pool = vec![create_json("imoge_123.json")];
        let mut config = Config::default();
        config.matching.levenshtein_threshold = 3;
        let matcher = Matcher::new(&json_pool, &config);

        let media = create_media(1, "image_123.jpg", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 7);
        assert_eq!(result.confidence, 90);
    }

    #[test]
    fn test_tier_7_dynamic_threshold() {
        let json_pool = vec![create_json("short1.json")];
        let mut config = Config::default();
        config.matching.levenshtein_threshold = 3;
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "short.jpg", ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 7);
        assert_eq!(result.json.filename, "short1.json");

        // dist 2 should fail
        let json_pool2 = vec![create_json("short12.json")];
        let matcher2 = Matcher::new(&json_pool2, &config);
        let result2 = matcher2.find_best_match(&media);
        assert!(result2.is_none());
    }

    #[test]
    fn test_tier_7_folder_cap() {
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
        assert_eq!(result.tier, 5);
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
        assert_eq!(result.tier, 6);
        assert_eq!(result.json.filename, truncated_json);
    }

    #[test]
    fn test_tier_6_unicode_progressive_truncation() {
        let long_unicode_stem = "照片_相册_旅行_2023_very_long_filename_string";
        let truncated_json = "照片_相册_旅行_2023_very.json";

        let json_pool = vec![create_json(truncated_json)];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, &format!("{}.jpg", long_unicode_stem), ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 6);
        assert_eq!(result.json.filename, truncated_json);
    }

    #[test]
    fn test_tier_6_emoji_progressive_truncation() {
        let long_emoji_stem = "📷_vacation_picture_2023_extra_long";
        let truncated_json = "📷_vacation_picture.json";

        let json_pool = vec![create_json(truncated_json)];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, &format!("{}.jpg", long_emoji_stem), ".jpg");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 6);
        assert_eq!(result.json.filename, truncated_json);
    }

    #[test]
    fn test_takeout_heic_duplicate_offset_index_2_to_1() {
        // Matches user scenario: IMG_1559(2).HEIC matching IMG_1559.HEIC.supplemental-metadata(1).json
        let json_pool = vec![
            create_json("IMG_1559.HEIC.supplemental-metadata.json"),
            create_json("IMG_1559.HEIC.supplemental-metadata(1).json"),
        ];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "IMG_1559(2).HEIC", ".heic");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 3);
        assert_eq!(
            result.json.filename,
            "IMG_1559.HEIC.supplemental-metadata(1).json"
        );
        assert_eq!(result.confidence, 100);
    }

    #[test]
    fn test_takeout_duplicate_multi_nested_parentheses() {
        // Matches user scenario: IMG_2072(1)(1).mov matching IMG_2072(1).mov.supplemental-metadata.json
        let json_pool = vec![
            create_json("IMG_2072(1).mov.supplemental-metadata.json"),
            create_json("IMG_2072.mov.supplemental-metadata(2).json"),
        ];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "IMG_2072(1)(1).mov", ".mov");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 3);
        assert_eq!(
            result.json.filename,
            "IMG_2072(1).mov.supplemental-metadata.json"
        );
        assert_eq!(result.confidence, 100);
    }

    #[test]
    fn test_takeout_duplicate_nested_with_zero() {
        // Matches user scenario: VID_20210809_151612(0)(1).mp4
        let json_pool = vec![create_json(
            "VID_20210809_151612(0).mp4.supplemental-metadata.json",
        )];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "VID_20210809_151612(0)(1).mp4", ".mp4");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 3);
        assert_eq!(
            result.json.filename,
            "VID_20210809_151612(0).mp4.supplemental-metadata.json"
        );
        assert_eq!(result.confidence, 100);
    }

    #[test]
    fn test_takeout_duplicate_fallback_to_base_unnumbered() {
        // Matches user scenario: IMG_3426(2).heic when only unnumbered base sidecar exists
        let json_pool = vec![create_json("IMG_3426.heic.supplemental-metadata.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "IMG_3426(2).heic", ".heic");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 3);
        assert_eq!(
            result.json.filename,
            "IMG_3426.heic.supplemental-metadata.json"
        );
        assert_eq!(result.confidence, 95);
    }

    #[test]
    fn test_live_photo_mov_with_nested_duplicate() {
        // Matches Live Photo MOV nested duplicate pairing with HEIC sidecar
        let json_pool = vec![create_json("IMG_2072(1).HEIC.supplemental-metadata.json")];
        let config = Config::default();
        let matcher = Matcher::new(&json_pool, &config);
        let media = create_media(1, "IMG_2072(1)(1).MOV", ".mov");

        let result = matcher.find_best_match(&media).unwrap();
        assert_eq!(result.tier, 5);
        assert_eq!(
            result.json.filename,
            "IMG_2072(1).HEIC.supplemental-metadata.json"
        );
        assert_eq!(result.confidence, 100);
    }
}

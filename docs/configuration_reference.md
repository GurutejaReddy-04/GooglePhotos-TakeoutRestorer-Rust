# Configuration Reference

The application reads its configuration from `config.toml` stored in the system's native configuration directory (e.g. `%APPDATA%\TakeoutRestorerTeam\GooglePhotosRestorer\config.toml` on Windows).

## `config.toml` Schema

```toml
version = 1

# Optional explicit path to the ExifTool binary.
# If omitted, the app will auto-download ExifTool.
# exiftool_path = "C:\\Tools\\exiftool.exe"

supported_image_extensions = [
    ".jpg", ".jpeg", ".png", ".heic", ".webp", ".gif", 
    ".tiff", ".tif", ".bmp", ".dng", ".cr2", ".nef", 
    ".arw", ".orf", ".rw2"
]

supported_video_extensions = [
    ".mp4", ".mov", ".mkv", ".webm", ".avi", ".wmv", ".flv"
]

[live_photo_pairs]
default_image_extension = ".jpg"
default_video_extension = ".mov"

[processing]
# Max concurrent background workers for ExifTool writes. Defaults to 4.
max_workers = 4

[matching]
# Max Levenshtein distance allowed to fuzzily match a media file to its JSON.
levenshtein_threshold = 3
# Min length of the filename required before attempting truncation matching (e.g. photo(1).jpg -> photo.json)
min_truncation_length = 8
```

## Environment Variables
- `RESTORER_MAX_WORKERS`: Overrides `processing.max_workers`.
- `RESTORER_LEVENSHTEIN_THRESHOLD`: Overrides `matching.levenshtein_threshold`.

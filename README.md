# Google Photos Takeout Restorer

An automated tool to restore metadata (Exif, GPS, timestamps) from Google Photos Takeout JSON files back into the original media files using ExifTool.

## Features
- **Intelligent Matching**: Uses Levenshtein distance and truncation logic to match JSONs to media files perfectly.
- **Auto-Healing**: Fixes incorrect file extensions (e.g. `.png` that is actually a `.jpg`).
- **Parallel Processing**: Heavily multithreaded architecture for rapid processing of massive takeout archives.
- **Event-Driven UI**: A highly responsive, resource-efficient Slint Graphical User Interface.

## Getting Started
Please consult the [Documentation Index](docs/README.md) for full manuals, or see the `docs/` folder for guides on building, deploying, and utilizing both the CLI and GUI tools.

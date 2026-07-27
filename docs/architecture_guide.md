# Architecture Guide

The Google Photos Takeout Restorer is architected using an Event-Driven Model-View-ViewModel (MVVM) pattern to strictly separate business logic from UI rendering.

## Core (`crates/core`)
- Owns all business truth.
- Analyzes files, matches JSON metadata using Levenshtein distance, and shells out to ExifTool for metadata writes.
- Uses `StateDatabase` (SQLite) for state persistence and resumability.
- Processes asynchronously using `Rayon`.

## Shared UI (`crates/shared_ui`)
- Maps internal `AppEvent`s into a `ProcessingSnapshot`.
- Guarantees thread safety via a debouncing background updater.

## App Orchestrator (`crates/app`)
- Provides `CoreDispatcher` implementing `CommandDispatcher`.
- Merges CLI and GUI execution paths into a unified binary.

## GUI (`crates/gui`)
- Implemented in Slint.
- Contains no direct references to the `core`.
- Only observes `ProcessingSnapshot` and dispatches `UiCommand`.

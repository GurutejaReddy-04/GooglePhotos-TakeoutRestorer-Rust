# CLI Reference

The application can be operated entirely from the terminal.

## Usage
`TakeoutRestorer [OPTIONS] <INPUTS>...`

## Arguments
- `<INPUTS>`: One or more Takeout directories or `.zip` files (required in CLI mode; optional if `--gui` is passed).

## Options
- `-o, --output <DIR>`: The target directory for restored files (required in CLI mode; optional if `--gui` is passed).
- `--db-path <PATH>`: Optional explicit path to the SQLite state database.
- `--use-system-exiftool`: Skips downloading ExifTool and attempts to use the system `exiftool` binary instead.
- `--gui`: Launches the graphical user interface.
- `-h, --help`: Print help information.
- `-V, --version`: Print version information.

## Example
`TakeoutRestorer --output C:\RestoredPhotos C:\Takeout\Takeout.zip`

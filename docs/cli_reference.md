# CLI Reference

The application can be operated entirely from the terminal.

## Usage
`TakeoutRestorer [OPTIONS] <INPUTS>...`

## Arguments
- `<INPUTS>`: One or more Takeout directories or `.zip` files.

## Options
- `-o, --output <DIR>`: The target directory for restored files. (Required)
- `--db-path <PATH>`: Optional explicit path to the SQLite state database.
- `--use-system-exiftool`: Skips downloading ExifTool and attempts to use the system `exiftool` binary instead.
- `--gui`: Launches the graphical user interface.

## Example
`TakeoutRestorer --output C:\RestoredPhotos C:\Takeout\Takeout.zip`

# Changelog

All notable changes to this project are listed here, newest first.

## unreleased
### added
- Export of reports to CSV.

## [2.1.0] - 2024-03-02
### Added
- Dark mode for the dashboard.
- Keyboard shortcuts for the search box.
### Fixed
- Totals were rounded twice on the summary page.

##Version 2.0.1 (2024-02-10)
#### fixed
- Login failed for user names that contain a plus sign.

## v2.0.0 — 15 January 2024
### CHANGED
- The API returns dates in ISO 8601 format.
- The settings moved to `config/app.toml`:

```toml
# where the reports are written
output_dir = "reports"
```

### Removed:
- The old XML export.

# 1.9.0 / 2023-12-01
## Added
- First public release.

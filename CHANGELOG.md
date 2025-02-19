# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1] - 2025-02-18
### Added
- Generate release notes from CHANGELOG.md.
- Cache build artifacts to speed up CI.

### Fixed
- Fix sending the initial packet one more time than necessary (even after ACK).

## [0.1.0] - 2025-02-17
Initial release.

### Added
- Add plugin to [Tera Term](https://teratermproject.github.io/index-en.html)
  terminal emulator that adds [LiteX](https://github.com/enjoy-digital/litex)
  [Serial Flash Loader](https://github.com/enjoy-digital/litex/wiki/Load-Application-Code-To-CPU#serial-boot) support.
  
  The plugin does adds the following:

  * Creates a file uploader dialog specifically for SFL parameters.
  * Waits for the SFL magic string from a device wishing to receive firmware.
  * Opens the file using the filename supplied bythe dialog.
  * Does a command-response lockstep send/receive with the device to send
    over a firmware at the device CPU address supplied by the dialog.

[Unreleased]: https://github.com/cr1901/teraterm-litex/compare/v0.1.1..HEAD
[0.1.1]: https://github.com/cr1901/teraterm-litex/releases/tag/v0.1.1
[0.1.0]: https://github.com/cr1901/teraterm-litex/releases/tag/v0.1.0

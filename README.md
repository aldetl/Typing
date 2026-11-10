# Typing

Typing is a Windows keyboard utility that helps you navigate and edit in the foreground application using configurable keyboard commands. It supports a Vim-like command mode and direct hotkeys across applications.

## Download the legacy application

**Typing 1.4.0 is the current legacy Windows release.**

- [Download Typing 1.4.0 for Windows](https://github.com/aldetl/Typing/releases/download/v1.4.0/Typing-1.4.0-legacy-windows.zip)
- [Release notes and SHA-256 checksum](https://github.com/aldetl/Typing/releases/tag/v1.4.0)

This release archives the existing application. It has not been rebuilt or changed for this publication.

### Getting started

1. Ensure that Microsoft .NET Framework 4.8 is available on your Windows system.
2. Download and extract the entire ZIP archive.
3. Open the extracted `Typing` folder and run `Start Typing.cmd` or `Typing.exe`.
4. Find Typing in the Windows notification area. The application starts in the tray.

Keep the executable, DLLs, configuration files, and `res` folder together. The included `Readme.md`, `configuration_readme.txt`, and `change log.md` describe the legacy application's operation and configuration. The bundled README still displays its older documentation version, 1.3.2; the executable's file and product versions are 1.4.0.0.

## Source publication and the next version

The current Rust implementation is being prepared for publication as **Typing 1.5.0**. Its source and release packages are not published in this repository yet.

InputSim, the separate input-simulation dependency used by the Rust implementation, is planned for its own repository. Source availability, build instructions, licensing terms, and migration guidance will accompany the new implementation when it is published.

The legacy application is distributed through GitHub Releases. The default branch starts with this README and will receive the new source separately.

## Contact

Author: [aldetl](https://github.com/aldetl)  
Email: freelssoft@gmail.com

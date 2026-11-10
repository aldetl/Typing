# Third-party components

Typing's combined Apache 2.0 with Commons Clause 1.0 license applies to Typing-owned material. It does not replace the licenses of the components listed below. The original Typing icon permissions are described separately in [ARTWORK-LICENSE.md](ARTWORK-LICENSE.md).

## InputSim

- Project: [aldetl/InputSim](https://github.com/aldetl/InputSim)
- Version: 0.1.0
- Source revision selected for this publication draft: `76158eb38423538e48d43919a71434ce2342bce1`
- License: Apache License 2.0, without Commons Clause.
- License text: [licenses/InputSim-LICENSE.txt](licenses/InputSim-LICENSE.txt).

InputSim is built from a separate checkout and is not vendored into the Typing source repository. Windows application packages include its matching DLL and must include its license text.

## Windows Rust bindings and import libraries

The locked Windows dependency graph uses `windows-sys` 0.52.0, `windows-targets` 0.52.5, and target-specific Windows import libraries. These components come from [microsoft/windows-rs](https://github.com/microsoft/windows-rs) and are licensed under MIT OR Apache-2.0.

The source lockfile records the resolved packages and checksums. Package production must retain the license notices from the actual components used for the selected target. The shared texts included here are [licenses/windows-rs-LICENSE-APACHE.txt](licenses/windows-rs-LICENSE-APACHE.txt) and [licenses/windows-rs-LICENSE-MIT.txt](licenses/windows-rs-LICENSE-MIT.txt); component-specific notices remain applicable.

## Rust standard library

Windows binaries also include Rust standard-library code. Preserve the copyright and third-party notices supplied by the actual Rust toolchain used for the build. The existing packaging script copies that toolchain's `COPYRIGHT-library.html` into the package's `licenses/` directory; this source snapshot does not substitute one toolchain's notice inventory for another's.

## Windows system libraries

Windows system DLLs are supplied by the operating system and are not copied into this source repository.

## Package notice inventory

Official Typing application distributions must carry Typing's `LICENSE`, `NOTICE`, and `ARTWORK-LICENSE.md`, this third-party overview, InputSim's license, the actual Windows dependency notices, and the Rust toolchain's notices. Unofficial distributions may satisfy the project-origin attribution condition by carrying the project's NOTICE or by showing the official Typing project and repository link in About, as specified in LICENSE. That choice does not replace other applicable software, artwork, copyright, Commons Clause, or third-party notice obligations. Review the required notices against the delivered archive before publication. Adding the source-level license files alone does not update the packaging script or an existing ZIP.

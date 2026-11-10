# Typing

Typing is an efficient tool that keeps your hands on the keyboard and reduces the need to switch between the keyboard and mouse while typing. You can use Vim-style command mode or Emacs-style direct shortcuts to move the caret, select text, and run custom commands across applications—without requiring plugins or integration from those applications.

## Features

- Keyboard navigation, text selection, editing shortcuts, movement counts, and date/time insertion.
- Independent mapping profiles with a visual editor, shortcut recording, and per-mapping enable/disable controls.
- Explicit profile activation: save a profile for later or apply it with **Set as active**.
- Tray controls, cursor feedback for command mode and Caps Lock, and optional startup when you sign in to Windows.
- A portable application with native Windows controls and an included user guide.

## Get started

Extract the complete portable package into a folder. Keep `Typing.exe`, `inputsim.dll`, `configuration.ini`, and `profiles/` together, and quit any other Typing or Aldetl instance using the same global shortcut.

1. Run `Typing.exe`. It starts enabled in the Windows notification area (the tray).
2. Open a text editor. With the default profile, tap and release **Caps Lock** or **Left Alt** alone to enter command mode.
3. Press **I / J / K / L** to move up / left / down / right. Hold a mode key with a configured shortcut, such as **Caps Lock+J**, to move directly without entering command mode first.
4. Tap a mode key again, or press **G**, to resume ordinary typing. Command mode also exits after inactivity by default.

Use **Ctrl+U** to enable or disable Typing. Choose **Show Window** from the tray to open the mapping editor, event diagnostics, settings, and help. Closing the window hides it; choose tray **Exit** to quit.

In **Mapping Profile**, **Save** writes a profile and **Set as active** applies it. **Import** brings in supported profiles or legacy configuration mappings without changing the source file. Review imported mappings before activating them. See the [user guide](docs/USER_GUIDE.md) for the full command list, profile management, preferences, and troubleshooting.

## Development

Typing is designed for Windows, macOS, and Linux. Windows is currently the only released platform; macOS and Linux support is under development.

### Windows architectures

| Build | Architecture | OS targets |
| --- | --- | --- |
| Windows x86 | 32-bit | Windows 7 SP1, Windows 10, and Windows 11 through WOW64 |
| Windows x64 | 64-bit | Windows 11 |

Development verification runs on Windows 11 x64. Actual Windows 7 SP1/Windows 10 execution and clean-machine deployment remain pending; these OS targets are not claims of completed deployment verification.

### Build from source

The Rust workspace separates command interpretation (`typing-core`), shared host coordination and profiles (`typing-host`), and native Windows integration (`typing-windows`). The separately maintained [InputSim](https://github.com/aldetl/InputSim) DLL supplies input simulation.

Use an x64 MSVC Rust host toolchain, Visual Studio or Build Tools with C++ tools for the selected architecture, and a Windows SDK. From the Typing repository root, prepare the adjacent InputSim checkout and locked dependencies, then build an x64 portable package:

```powershell
git clone https://github.com/aldetl/InputSim.git ../inputsim
git -C ../inputsim checkout 76158eb38423538e48d43919a71434ce2342bce1
cargo fetch --locked --target x86_64-pc-windows-msvc
cargo fetch --manifest-path ../inputsim/Cargo.toml --locked --target x86_64-pc-windows-msvc
.\tools\package_windows.ps1 -Architecture x64
```

The packaging script builds both projects with locked, offline dependencies and writes the package and integrity manifest under `target/dist/`. The application and DLL must have matching architectures.

For Windows x86 with Windows 7 compatibility, use `nightly-2026-04-15`, `rust-src`, and the `i686-win7-windows-msvc` target with a rebuilt standard library. Provision that toolchain and its locked dependencies before running `.\tools\package_windows.ps1 -Architecture x86`.

### Validation

Run source/build checks and compile the native test tools with:

```powershell
.\tools\validate_windows.ps1 -BuildOnly -Architecture x64
```

Use `-Architecture x86` for the additional x86 checks. Use `-Portable` instead of `-BuildOnly` to run desktop checks against an extracted package. Desktop checks require an interactive Windows session and can operate application windows and inject test input.

## License

Typing is source-available under [Apache License 2.0 with Commons Clause 1.0](LICENSE). Personal and workplace use are permitted under those terms; the Commons Clause restriction on selling applies.

Unofficial distributions must include [NOTICE](NOTICE) or identify **Typing** and link to its [official repository](https://github.com/aldetl/Typing) in About. This attribution also permits reuse of the Typing icons; no separate permission request is needed.

InputSim is independently licensed under Apache 2.0 alone. Other dependencies retain their own terms; see [third-party notices](THIRD_PARTY_NOTICES.md).

[Website](https://xdetail.org/typing) · [Downloads](https://github.com/aldetl/Typing/releases) · [User guide](docs/USER_GUIDE.md)

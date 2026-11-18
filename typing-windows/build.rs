use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=res");
    if env::var("CARGO_CFG_TARGET_OS").unwrap() != "windows" {
        return;
    }
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("res");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let resource = out.join("typing.rc");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let numeric: Vec<u16> = version
        .split('.')
        .map(|part| part.parse().unwrap())
        .collect();
    assert_eq!(numeric.len(), 3, "Windows version needs major.minor.patch");
    let numeric = format!("{},{},{},0", numeric[0], numeric[1], numeric[2]);
    let path = |name: &str| source.join(name).to_string_lossy().replace('\\', "/");
    fs::write(
        &resource,
        format!(
            "1 24 \"{}\"\n100 ICON \"{}\"\n101 ICON \"{}\"\n102 ICON \"{}\"\n103 ICON \"{}\"\n\
             1 VERSIONINFO\nFILEVERSION {numeric}\nPRODUCTVERSION {numeric}\n\
             FILEFLAGSMASK 0x3fL\nFILEFLAGS 0\nFILEOS 0x40004L\nFILETYPE 0x1L\nFILESUBTYPE 0\n\
             BEGIN\nBLOCK \"StringFileInfo\"\nBEGIN\nBLOCK \"040904b0\"\nBEGIN\n\
             VALUE \"FileDescription\", \"TypingVE\\0\"\n\
             VALUE \"ProductName\", \"TypingVE\\0\"\n\
             VALUE \"InternalName\", \"TypingVE.exe\\0\"\n\
             VALUE \"OriginalFilename\", \"TypingVE.exe\\0\"\n\
             VALUE \"FileVersion\", \"{version}\\0\"\n\
             VALUE \"ProductVersion\", \"{version}\\0\"\n\
             END\nEND\nBLOCK \"VarFileInfo\"\nBEGIN\nVALUE \"Translation\", 0x409, 1200\nEND\nEND\n",
            path("typing.manifest"),
            path("main.ico"),
            path("icon1.ico"),
            path("icon2.ico"),
            path("icon3.ico")
        ),
    )
    .unwrap();
    let sdk = PathBuf::from(env::var_os("ProgramFiles(x86)").unwrap()).join("Windows Kits/10/bin");
    let mut versions: Vec<_> = fs::read_dir(sdk)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("x64/rc.exe").exists())
        .collect();
    versions.sort();
    let compiler = versions
        .last()
        .expect("Windows SDK resource compiler is required")
        .join("x64/rc.exe");
    let compiled = out.join("typing.res");
    let status = Command::new(compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&compiled)
        .arg(&resource)
        .status()
        .unwrap();
    assert!(status.success(), "resource compilation failed");
    println!("cargo:rustc-link-arg-bin=TypingVE={}", compiled.display());
}

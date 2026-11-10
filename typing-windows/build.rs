use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=res");
    if env::var("CARGO_CFG_TARGET_OS").unwrap() != "windows" {
        return;
    }
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("res");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let resource = out.join("typing.rc");
    let path = |name: &str| source.join(name).to_string_lossy().replace('\\', "/");
    fs::write(
        &resource,
        format!(
            "1 24 \"{}\"\n100 ICON \"{}\"\n101 ICON \"{}\"\n102 ICON \"{}\"\n103 ICON \"{}\"\n",
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
    println!("cargo:rustc-link-arg-bin=Typing={}", compiled.display());
}

//! 编译期把 app.manifest 和 resources（图标 + 版本信息）链进 exe。
//!
//! 两件事都走 link.exe / rc.exe 的命令行，不引第三方 build-dependency
//! （embed-resource / winres）：各一条参数就够，少一级编译、少一个依赖树。
//!
//! 找不到 rc.exe 时**不中断构建**：只打一条 warning，exe 退化成"没有图标、
//! 没有版本信息"的版本。这样在没有装 Windows SDK 的环境里也能编译，
//! 代价只是产物不完整 —— 比直接编译失败友好得多。

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("app.manifest");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");

    compile_resources();
}

fn compile_resources() {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let rc_file = assets.join("app.rc");
    for f in ["app.rc", "app.ico", "tray-dark.ico", "tray-light.ico"] {
        println!("cargo:rerun-if-changed={}", assets.join(f).display());
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let res = out_dir.join("sniptool.res");

    let Some(rc) = find_rc() else {
        println!(
            "cargo:warning=rc.exe 未找到（需要 Windows SDK）。跳过图标与版本信息，产物将是一个没有图标的 exe。"
        );
        return;
    };

    let status = Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg(&rc_file)
        .status();

    match status {
        Ok(s) if s.success() => {
            // .res 作为链接器输入直接传下去：link.exe 按扩展名识别
            println!("cargo:rustc-link-arg={}", res.display());
        }
        Ok(s) => println!("cargo:warning=rc.exe 失败（{s}），图标与版本信息未嵌入"),
        Err(e) => println!("cargo:warning=无法执行 rc.exe：{e}"),
    }
}

/// 找 rc.exe：先看环境变量 RC，再看 PATH，最后扫 Windows SDK 目录。
fn find_rc() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RC") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if Command::new("rc.exe")
        .arg("/?")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
    {
        return Some(PathBuf::from("rc.exe"));
    }

    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "x64",
    };
    let roots = [
        std::env::var("ProgramFiles(x86)").ok(),
        std::env::var("ProgramFiles").ok(),
    ];
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for root in roots.into_iter().flatten() {
        let bin = Path::new(&root).join("Windows Kits").join("10").join("bin");
        let Ok(entries) = std::fs::read_dir(&bin) else {
            continue;
        };
        for e in entries.flatten() {
            let ver: Vec<u32> = e
                .file_name()
                .to_string_lossy()
                .split('.')
                .filter_map(|s| s.parse::<u32>().ok())
                .collect();
            let candidate = e.path().join(arch).join("rc.exe");
            if candidate.is_file() && best.as_ref().map_or(true, |(v, _)| ver > *v) {
                best = Some((ver, candidate));
            }
        }
    }
    best.map(|(_, p)| p)
}

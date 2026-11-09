//! 纯 Rust 的 7z 解压工具（基于 sevenz-rust），用于在没有安装 7-Zip 的系统上
//! 解出 Umi-OCR 的模型包，例如打包流程中的 `models.7z`。
//!
//! 用法：`unpack7z <archive.7z> <输出目录>`

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(archive), Some(out_dir)) = (args.next(), args.next()) else {
        eprintln!("用法: unpack7z <archive.7z> <输出目录>");
        return ExitCode::from(2);
    };

    let out = Path::new(&out_dir);
    if let Err(err) = std::fs::create_dir_all(out) {
        eprintln!("创建输出目录失败 {out_dir}: {err}");
        return ExitCode::FAILURE;
    }

    match sevenz_rust::decompress_file(&archive, out) {
        Ok(()) => {
            println!("已解压: {archive} -> {out_dir}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("解压失败 {archive}: {err}");
            ExitCode::FAILURE
        }
    }
}

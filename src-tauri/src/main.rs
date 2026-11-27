#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--hook-gate") {
        if let Err(err) = lumen_lib::hook_gate::run(&args) {
            eprintln!("hook gate error: {err}");
            std::process::exit(2);
        }
        return;
    }

    #[cfg(windows)]
    {
        // AppContainer 沙箱的隐藏模式：由主进程以普通用户身份拉起。
        if let Some(path) = arg_value(&args, "--sandbox-host") {
            let code = lumen_lib::sandbox::host::run(std::path::Path::new(&path));
            std::process::exit(code);
        }
        if let Some(path) = arg_value(&args, "--sandbox-relay") {
            if let Err(err) = lumen_lib::sandbox::relay::run_relay(std::path::Path::new(&path)) {
                eprintln!("sandbox relay error: {err}");
                std::process::exit(2);
            }
            return;
        }
        if let Some(path) = arg_value(&args, "--sandbox-selftest") {
            let code = lumen_lib::sandbox::probe::run_selftest(std::path::Path::new(&path));
            std::process::exit(code);
        }
        // 诊断/卸载用：回收沙箱已授予的 ACL 与运行时 profile。
        if let Some(dir) = arg_value(&args, "--sandbox-cleanup") {
            match lumen_lib::sandbox::session::cleanup(
                std::path::Path::new(&dir),
                lumen_lib::sandbox::session::RUN_PROFILE,
            ) {
                Ok(count) => {
                    println!("sandbox cleanup: {count} acl records removed");
                    return;
                }
                Err(err) => {
                    eprintln!("sandbox cleanup error: {err}");
                    std::process::exit(2);
                }
            }
        }
        // 诊断用：在命令行直接跑一遍完整自检，输出 JSON。
        if let Some(dir) = arg_value(&args, "--sandbox-probe") {
            match lumen_lib::sandbox::probe::run(std::path::Path::new(&dir)) {
                Ok(report) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&report).unwrap_or_default()
                    );
                    std::process::exit(if report.ok { 0 } else { 3 });
                }
                Err(err) => {
                    eprintln!("sandbox probe error: {err}");
                    std::process::exit(3);
                }
            }
        }
        // 诊断用：在真实项目目录中运行一次指定 CLI，输出 JSON 报告。
        if let Some(path) = arg_value(&args, "--sandbox-exec") {
            match lumen_lib::sandbox::exec::run(std::path::Path::new(&path)) {
                Ok(report) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&report).unwrap_or_default()
                    );
                    std::process::exit(if report.ok { 0 } else { 3 });
                }
                Err(err) => {
                    eprintln!("sandbox exec error: {err}");
                    std::process::exit(3);
                }
            }
        }
    }

    lumen_lib::run();
}

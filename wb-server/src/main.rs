//! `wb-server --preset earthlike [--port 8787] [--static DIR] [--param k=v]... [--solve]`
use std::path::PathBuf;
use wb_server::{start, ServerConfig};

fn usage() -> ! {
    eprintln!("usage: wb-server [--preset NAME] [--port N (0 = any free port)] [--static DIR] [--param k=v]... [--solve]");
    eprintln!("  --preset   earthlike | desert | waterworld | tidally_locked | ice_age   (default earthlike)");
    eprintln!("  --static   directory with the built client (default: ui-web/dist found from the current dir or the executable)");
    eprintln!("  --param    start with a parameter changed, e.g. --param resolution=4 (repeatable)");
    eprintln!("  --solve    start a solve as soon as the server is up");
    std::process::exit(2)
}

fn find_dist() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = vec![];
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        roots.extend(exe.ancestors().skip(1).take(5).map(|p| p.to_path_buf()));
    }
    roots.into_iter().map(|r| r.join("ui-web").join("dist")).find(|p| p.join("index.html").is_file())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = ServerConfig::default();
    let mut static_dir: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].clone();
        let mut next = || {
            i += 1;
            args.get(i).cloned().unwrap_or_else(|| usage())
        };
        match flag.as_str() {
            "--preset" => cfg.preset = next(),
            "--port" => cfg.port = next().parse().unwrap_or_else(|_| usage()),
            "--static" => static_dir = Some(PathBuf::from(next())),
            "--param" => {
                let kv = next();
                let (k, v) = kv.split_once('=').unwrap_or_else(|| usage());
                cfg.params.push((k.to_string(), v.parse::<f64>().unwrap_or_else(|_| usage())));
            }
            "--solve" => cfg.solve_on_start = true,
            "--step-delay-ms" => cfg.step_delay = std::time::Duration::from_millis(next().parse().unwrap_or_else(|_| usage())),
            "--help" | "-h" => usage(),
            _ => usage(),
        }
        i += 1;
    }
    cfg.static_dir = static_dir.or_else(find_dist);
    let served = cfg.static_dir.clone();
    let mut h = match start(cfg) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("wb-server: cannot start: {e}");
            std::process::exit(1)
        }
    };
    println!("wb-server listening on {} (localhost only)", h.url());
    println!("  websocket: {}", h.ws_url());
    match served {
        Some(d) => println!("  client files: {}", d.display()),
        None => println!("  client files: not found (build with `npm run build -w ui-web`, or pass --static DIR)"),
    }
    h.wait();
}

//! wb-snap: solve a preset and write a flat-projection PNG of one field.
//! `wb-snap --preset earthlike --field precipitation --out rain.png [--param name=value ...] [--width 720] [--layer 0] [--no-legend]`
use wb_app::raster::{render_field, value_range, RenderOpts};
use wb_app::{png::fnv1a, Command, Query, Session};

fn usage() -> ! {
    eprintln!("usage: wb-snap --preset <name> --field <name> --out <file.png> [--param k=v]... [--width N] [--layer N] [--no-legend] [--list]");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut preset, mut field, mut out) = ("earthlike".to_string(), "temperature".to_string(), "snap.png".to_string());
    let (mut params, mut opts, mut list) = (vec![], RenderOpts::default(), false);
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].clone();
        let mut next = || { i += 1; args.get(i).cloned().unwrap_or_else(|| usage()) };
        match flag.as_str() {
            "--preset" => preset = next(),
            "--field" => field = next(),
            "--out" => out = next(),
            "--width" => opts.width = next().parse().unwrap_or_else(|_| usage()),
            "--layer" => opts.layer = next().parse().unwrap_or_else(|_| usage()),
            "--param" => {
                let kv = next();
                let (k, v) = kv.split_once('=').unwrap_or_else(|| usage());
                params.push((k.to_string(), v.parse::<f64>().unwrap_or_else(|_| usage())));
            }
            "--no-legend" => opts.legend = false,
            "--list" => list = true,
            _ => usage(),
        }
        i += 1;
    }
    let fail = |e: wb_app::ApiError| -> ! { eprintln!("error: {e}"); std::process::exit(1) };
    let mut s = Session::with_preset(&preset).unwrap_or_else(|e| fail(e));
    for (k, v) in params {
        s.execute(Command::SetParam { name: k, value: v }).unwrap_or_else(|e| fail(e));
    }
    if list {
        let info = s.query(Query::GetWorldInfo).unwrap_or_else(|e| fail(e));
        println!("{}", serde_json::to_string_pretty(&info.json["fields"]).unwrap());
        return;
    }
    if field != "elevation" {
        let t = std::time::Instant::now();
        let status = s.solve_blocking().unwrap_or_else(|e| fail(e));
        eprintln!("solve {status} in {:.2}s", t.elapsed().as_secs_f64());
    }
    let img = render_field(&s, &field, &opts).unwrap_or_else(|e| fail(e));
    let png = img.to_png();
    std::fs::write(&out, &png).unwrap_or_else(|e| { eprintln!("cannot write {out}: {e}"); std::process::exit(1) });
    let f = s.field(&field, opts.layer).unwrap_or_else(|e| fail(e));
    let (lo, hi) = value_range(&field, &f.values);
    let meta = wb_app::schema::field_json(&field);
    println!("{out}: {}x{} px, field {field} ({}), colour range {lo:.4e}..{hi:.4e} {}, png hash {:016x}", img.width, img.height, meta["colormap"].as_str().unwrap_or(""), meta["unit"].as_str().unwrap_or(""), fnv1a(&png));
}

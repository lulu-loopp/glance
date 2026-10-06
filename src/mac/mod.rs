//! Glance on macOS. So far its readings, which `glance --sample [count]`
//! prints, a sample a second, for checking against Activity Monitor.

mod hid;
mod iokit;
mod ioreport;
mod sampler;
mod smc;

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    match args.as_slice() {
        [_, flag, rest @ ..] if flag == "--sample" => {
            let count = rest.first().and_then(|n| n.parse().ok()).unwrap_or(3);
            sampler::print(count);
        }
        _ => {
            eprintln!("Glance for macOS is on its way; for now, `glance --sample` prints what it reads.");
            std::process::exit(1);
        }
    }
}

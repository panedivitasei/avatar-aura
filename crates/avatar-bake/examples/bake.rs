//! Runs the bake command line from avatar_export.cpp's entry points: `cargo run --example bake -- --avatar ...`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match avatar_bake::cli::run(&args) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}

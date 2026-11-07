// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `edgepass init|validate|doctor|migrate` answer on the command line; with no
    // (or unknown) arguments the widget opens, which is the product. See `cli.rs`.
    let args: Vec<String> = std::env::args().collect();
    if let Some(result) = edgepass_lib::cli_main(&args) {
        match result {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("edgepass: {e}");
                std::process::exit(1);
            }
        }
    }
    edgepass_lib::run();
}

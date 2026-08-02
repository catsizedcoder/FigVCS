fn main() {
    if let Err(err) = fvcs::cli::run() {
        eprintln!("fvcs: error: {err:#}");
        std::process::exit(1);
    }
}

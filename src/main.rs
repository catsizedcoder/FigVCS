mod cli;
mod commit;
mod diff;
mod ignore;
mod index;
mod object;
mod refs;
mod repo;
mod status;

fn main() {
    if let Err(err) = cli::run() {
        eprintln!("fvcs: error: {err:#}");
        std::process::exit(1);
    }
}

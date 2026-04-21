use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let output = gitscale::run_cli(&args_ref);

    print!("{}", output.stdout);
    eprint!("{}", output.stderr);

    if !output.success {
        process::exit(1);
    }
}

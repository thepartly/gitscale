//! `git topic`: git runs this for `git topic ...`; the same as `git scale topic ...`.

fn main() {
    std::process::exit(gitscale::main(Some("topic")));
}

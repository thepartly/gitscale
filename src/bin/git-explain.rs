//! `git explain`: git runs this for `git explain ...`; the same as `git scale explain ...`.

fn main() {
    std::process::exit(gitscale::main(Some("explain")));
}

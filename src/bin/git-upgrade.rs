//! `git upgrade`: git runs this for `git upgrade ...`; the same as `git scale upgrade ...`.

fn main() {
    std::process::exit(gitscale::main(Some("upgrade")));
}

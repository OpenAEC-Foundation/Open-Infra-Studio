//! Onschuldig Windows-programma: leg uitsluitend vast hoe het is gestart.
use std::{env, fs, path::PathBuf};

fn main() {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() == 2 && args[0] == "--viewer" {
        let file = PathBuf::from(&args[1]);
        fs::write(file.with_extension("viewed"), file.to_str().unwrap()).unwrap();
    } else {
        fs::write(env::current_exe().unwrap().with_extension("executed"), b"uitgevoerd").unwrap();
    }
}

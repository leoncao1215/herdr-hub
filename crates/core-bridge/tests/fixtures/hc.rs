use std::io::{self, Write};

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(args.len(), 3);
    assert_eq!(&args[..2], ["connect", "--session"]);
    eprintln!("test fixture diagnostic");
    match args[2].as_str() {
        "echo" => {
            io::copy(&mut io::stdin().lock(), &mut io::stdout().lock()).unwrap();
            io::stdout().write_all(b"after-eof").unwrap();
        }
        "closed" => io::stdout().write_all(b"done").unwrap(),
        "fail" => {
            io::stdout().write_all(b"before-failure").unwrap();
            std::process::exit(17);
        }
        "blocked" => {
            io::stdout().write_all(b"ready").unwrap();
            io::stdout().flush().unwrap();
            loop {
                std::thread::park();
            }
        }
        _ => panic!("unknown test fixture mode"),
    }
}

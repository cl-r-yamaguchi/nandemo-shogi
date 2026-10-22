mod config;

fn main() {
    match config::Config::from_env() {
        Ok(config) => println!("設定を読み込みました: {config:?}"),
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}

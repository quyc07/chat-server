use std::process::ExitCode;

const DEFAULT_SERVER: &str = "http://127.0.0.1:3000";

fn main() -> ExitCode {
    let server = match parse_args(std::env::args().skip(1)) {
        Ok(server) => server,
        Err(message) => {
            eprintln!("chat-tui: {message}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if server == "help" {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("chat-tui: cannot start async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(chat_tui::app::run(server)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("chat-tui: {error}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
Usage: chat-tui [--server <url>]

  --server <url>   chat server base URL (default: http://127.0.0.1:3000)
  -h, --help       show this help

Keys: Tab/Up/Down move, Enter submit, Ctrl-R/F2 login/register, q/Esc/Ctrl-C quit.";

fn parse_args(args: impl Iterator<Item = String>) -> Result<String, String> {
    let mut server = DEFAULT_SERVER.to_string();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok("help".to_string()),
            "--server" => {
                server = args
                    .next()
                    .ok_or_else(|| "--server requires a URL".to_string())?;
            }
            other if other.starts_with("--server=") => {
                server = other["--server=".len()..].to_string();
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if !server.starts_with("http://") && !server.starts_with("https://") {
        return Err(format!("--server must be an http(s) URL, got {server:?}"));
    }
    Ok(server)
}

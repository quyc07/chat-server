use std::process::ExitCode;

const DEFAULT_SERVER: &str = "http://127.0.0.1:3000";

fn main() -> ExitCode {
    let server = match parse_args(std::env::args().skip(1)) {
        Ok(Args::Run(server)) => server,
        Ok(Args::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("chat-tui: {message}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

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

Keys: Tab/Up/Down move, Enter submit/send, Ctrl-R/F2 login/register or reload,
Esc/Ctrl-C quit. On the conversation screen q is ordinary message text.";

/// What the command line asked for. `Help` is its own outcome so no sentinel
/// string can ever be mistaken for a server URL.
enum Args {
    Run(String),
    Help,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut server = DEFAULT_SERVER.to_string();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Args::Help),
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
    Ok(Args::Run(server))
}

#[cfg(test)]
mod tests {
    use super::{Args, parse_args};

    fn parse(args: &[&str]) -> Result<Args, String> {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn help_is_its_own_outcome_not_a_sentinel_server_url() {
        assert!(matches!(parse(&["--help"]), Ok(Args::Help)));
        assert!(matches!(parse(&["-h"]), Ok(Args::Help)));
    }

    #[test]
    fn a_server_url_is_the_default_or_the_given_one() {
        assert!(matches!(
            parse(&[]),
            Ok(Args::Run(server)) if server == "http://127.0.0.1:3000"
        ));
        assert!(matches!(
            parse(&["--server", "http://example.test:8080"]),
            Ok(Args::Run(server)) if server == "http://example.test:8080"
        ));
        assert!(matches!(
            parse(&["--server=https://example.test"]),
            Ok(Args::Run(server)) if server == "https://example.test"
        ));
    }

    #[test]
    fn a_non_http_url_and_an_unknown_flag_are_errors() {
        assert!(parse(&["--server", "localhost:3000"]).is_err());
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["--server"]).is_err());
    }
}

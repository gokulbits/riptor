//! Controls a running `riptord`.

#[cfg(not(unix))]
fn main() {
    eprintln!("riptor-cli runs on Linux. On Windows, use the Riptor app.");
    std::process::exit(1);
}

#[cfg(unix)]
fn main() {
    if let Err(e) = unix::run() {
        eprintln!("riptor-cli: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(unix)]
mod unix {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    use anyhow::{bail, Context};
    use riptor_daemon::{Request, Response};

    const USAGE: &str = "riptor-cli list
riptor-cli stats
riptor-cli add <magnet | https link | /path/file.torrent> [--dir FOLDER] [--category NAME] [--paused]
riptor-cli stop <id...|all>
riptor-cli start <id...|all>
riptor-cli remove <id...> [--delete-files]
riptor-cli shutdown

Ids are info hashes; 6 or more leading characters are enough.";

    fn request(args: &[String]) -> anyhow::Result<Request> {
        let (cmd, rest) = args.split_first().context(USAGE)?;
        let flag = |name: &str| rest.iter().any(|a| a == name);
        let value = |name: &str| -> anyhow::Result<Option<String>> {
            match rest.iter().position(|a| a == name) {
                Some(i) => Ok(Some(
                    rest.get(i + 1)
                        .cloned()
                        .with_context(|| format!("{name} needs a value"))?,
                )),
                None => Ok(None),
            }
        };
        let ids = || -> Vec<String> {
            rest.iter()
                .filter(|a| !a.starts_with("--"))
                .cloned()
                .collect()
        };
        Ok(match cmd.as_str() {
            "list" | "ls" => Request::List,
            "stats" => Request::Stats,
            "add" => {
                let source = rest
                    .first()
                    .filter(|s| !s.starts_with("--"))
                    .cloned()
                    .context(USAGE)?;
                // Relative paths mean nothing to the daemon.
                let source = if !source.contains(':') && !source.starts_with('/') {
                    std::env::current_dir()?
                        .join(&source)
                        .to_string_lossy()
                        .into_owned()
                } else {
                    source
                };
                Request::Add {
                    source,
                    dir: value("--dir")?
                        .map(|d| std::env::current_dir().map(|c| c.join(d)))
                        .transpose()?,
                    category: value("--category")?,
                    paused: flag("--paused"),
                }
            }
            "stop" | "pause" => Request::Stop { ids: ids() },
            "start" | "resume" => Request::Start { ids: ids() },
            "remove" | "rm" => Request::Remove {
                ids: ids(),
                delete_files: flag("--delete-files"),
            },
            "shutdown" => Request::Shutdown,
            "-h" | "--help" | "help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown command {other}\n\n{USAGE}"),
        })
    }

    fn rate(bps: u64) -> String {
        match bps {
            0 => "-".into(),
            b if b < 1024 * 1024 => format!("{:.0} KiB/s", b as f64 / 1024.0),
            b => format!("{:.1} MiB/s", b as f64 / 1024.0 / 1024.0),
        }
    }

    fn print(req: &Request, data: &serde_json::Value) {
        match req {
            Request::List => {
                let list = data.as_array().cloned().unwrap_or_default();
                if list.is_empty() {
                    println!("No torrents.");
                }
                println!(
                    "{:<10} {:<12} {:>6} {:>11} {:>11}  NAME",
                    "ID", "STATE", "DONE", "DOWN", "UP"
                );
                for t in list {
                    let s = |k: &str| t[k].as_str().unwrap_or("").to_string();
                    let n = |k: &str| t[k].as_u64().unwrap_or(0);
                    println!(
                        "{:<10} {:<12} {:>5.1}% {:>11} {:>11}  {}",
                        s("id").chars().take(10).collect::<String>(),
                        s("state"),
                        t["progress"].as_f64().unwrap_or(0.0) * 100.0,
                        rate(n("dl_rate")),
                        rate(n("ul_rate")),
                        s("name")
                    );
                }
            }
            Request::Stats => {
                println!("network   {}", data["network"].as_str().unwrap_or("?"));
                println!("down      {}", rate(data["dl_rate"].as_u64().unwrap_or(0)));
                println!("up        {}", rate(data["ul_rate"].as_u64().unwrap_or(0)));
                if let Some(p) = data["listen_port"].as_u64() {
                    println!("port      {p}");
                }
            }
            Request::Add { .. } => println!("added {}", data["id"].as_str().unwrap_or("")),
            _ => println!("ok"),
        }
    }

    pub fn run() -> anyhow::Result<()> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let req = request(&args)?;
        let sock: PathBuf = riptor_daemon::socket_path();
        let mut stream = UnixStream::connect(&sock).with_context(|| {
            format!("can't reach riptord at {} (is it running?)", sock.display())
        })?;
        let mut line = serde_json::to_vec(&req)?;
        line.push(b'\n');
        stream.write_all(&line)?;
        let mut reply = String::new();
        BufReader::new(&stream).read_line(&mut reply)?;
        let resp: Response = serde_json::from_str(&reply).context("riptord sent a bad reply")?;
        if !resp.ok {
            bail!("{}", resp.error.unwrap_or_else(|| "failed".into()));
        }
        print(&req, &resp.data);
        Ok(())
    }
}

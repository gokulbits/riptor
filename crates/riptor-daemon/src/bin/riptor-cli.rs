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
    use riptor_daemon::cli::{self, Parsed};
    use riptor_daemon::{Request, Response};

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
        let req = match cli::parse(&args, &std::env::current_dir()?)? {
            Parsed::Run(req) => req,
            Parsed::Help(text) => {
                println!("{text}");
                return Ok(());
            }
        };
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

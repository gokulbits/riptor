//! Headless Riptor: the engine without a window, controlled with `riptor-cli`.

#[cfg(not(unix))]
fn main() {
    eprintln!("riptord runs on Linux. On Windows, use the Riptor app.");
    std::process::exit(1);
}

#[cfg(unix)]
fn main() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(unix::run())
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use anyhow::{bail, ensure, Context};
    use riptor_core::{AddParams, Engine, EngineConfig, Source};
    use riptor_daemon::{resolve_ids, Request, Response, MAX_LINE};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{UnixListener, UnixStream};
    use tracing::{info, warn};

    fn args() -> anyhow::Result<PathBuf> {
        let mut it = std::env::args().skip(1);
        let mut data = None;
        while let Some(a) = it.next() {
            match a.as_str() {
                "--data-dir" => {
                    data = Some(PathBuf::from(
                        it.next().context("--data-dir needs a folder")?,
                    ))
                }
                "-h" | "--help" => {
                    println!("riptord [--data-dir FOLDER]\nControl it with riptor-cli.");
                    std::process::exit(0);
                }
                other => bail!("unknown option {other}"),
            }
        }
        let dir = match data {
            Some(d) => d,
            None => riptor_daemon::default_data_dir()?,
        };
        ensure!(dir.is_absolute(), "--data-dir must be an absolute path");
        Ok(dir)
    }

    /// A folder only we can enter, never a link someone planted.
    fn private_dir(dir: &Path) -> anyhow::Result<()> {
        match std::fs::symlink_metadata(dir) {
            Ok(m) => {
                ensure!(
                    m.is_dir() && !m.file_type().is_symlink(),
                    "{} is not a plain folder",
                    dir.display()
                );
                ensure!(
                    m.uid() == rustix::process::getuid().as_raw(),
                    "{} belongs to another user",
                    dir.display()
                );
            }
            Err(_) => std::fs::create_dir_all(dir)?,
        }
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        Ok(())
    }

    pub async fn run() -> anyhow::Result<()> {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info,librqbit=warn".into()),
            )
            .with_writer(std::io::stderr)
            .init();
        let data_dir = args()?;
        // Falls back to the built-in key where there is no keyring (most servers).
        riptor_core::history::use_os_key();
        let engine = Engine::start(EngineConfig {
            data_dir: data_dir.clone(),
        })
        .await?;
        if engine.kill_switch_on() {
            warn!("the kill switch is on, but headless mode has no VPN: torrents stay held");
        }

        let dir = riptor_daemon::socket_dir();
        private_dir(&dir)?;
        let sock = riptor_daemon::socket_path();
        if sock.exists() {
            ensure!(
                UnixStream::connect(&sock).await.is_err(),
                "riptord is already running"
            );
            std::fs::remove_file(&sock)?;
        }
        let listener = UnixListener::bind(&sock)?;
        std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))?;
        info!(
            "riptord ready; data in {}, control at {}",
            data_dir.display(),
            sock.display()
        );

        // Remote control (qBittorrent Web API), when turned on in the settings.
        tokio::spawn(riptor_webapi::supervise(
            engine.clone(),
            data_dir.clone(),
            Default::default(),
        ));
        let (stop_tx, mut stop_rx) = tokio::sync::mpsc::channel::<()>(1);
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    let (engine, stop_tx) = (engine.clone(), stop_tx.clone());
                    tokio::spawn(async move {
                        if let Err(e) = serve(stream, &engine, &stop_tx).await {
                            warn!("control connection: {e:#}");
                        }
                    });
                }
                _ = stop_rx.recv() => break,
                _ = tokio::signal::ctrl_c() => break,
                _ = term.recv() => break,
            }
        }
        info!("shutting down");
        let _ = std::fs::remove_file(&sock);
        engine.shutdown().await?;
        Ok(())
    }

    async fn serve(
        stream: UnixStream,
        engine: &Arc<Engine>,
        stop: &tokio::sync::mpsc::Sender<()>,
    ) -> anyhow::Result<()> {
        let (r, mut w) = stream.into_split();
        let mut lines = BufReader::new(r.take(MAX_LINE as u64 * 16)).lines();
        while let Some(line) = lines.next_line().await? {
            ensure!(line.len() <= MAX_LINE, "request too long");
            let resp = match serde_json::from_str::<Request>(&line) {
                Ok(Request::Shutdown) => {
                    let _ = stop.send(()).await;
                    Response::ok(serde_json::Value::Null)
                }
                Ok(req) => match handle(engine, req).await {
                    Ok(v) => Response::ok(v),
                    Err(e) => Response::err(&e),
                },
                Err(e) => Response::err(&anyhow::anyhow!("bad request: {e}")),
            };
            let mut out = serde_json::to_vec(&resp)?;
            out.push(b'\n');
            w.write_all(&out).await?;
        }
        Ok(())
    }

    fn known(engine: &Engine) -> Vec<String> {
        engine.snapshot().0.into_iter().map(|t| t.id).collect()
    }

    async fn handle(engine: &Arc<Engine>, req: Request) -> anyhow::Result<serde_json::Value> {
        Ok(match req {
            Request::List => serde_json::to_value(engine.snapshot().0)?,
            Request::Stats => serde_json::to_value(engine.snapshot().1)?,
            Request::Add {
                source,
                dir,
                category,
                paused,
            } => {
                let source = source.trim().to_string();
                let src = if source.starts_with("magnet:") {
                    Source::Magnet(source)
                } else if source.starts_with("http://") || source.starts_with("https://") {
                    Source::Url(source)
                } else {
                    let p = PathBuf::from(&source);
                    ensure!(p.is_absolute(), "give the .torrent file's full path");
                    Source::File(p)
                };
                let id = engine
                    .add_direct(
                        src,
                        AddParams {
                            save_path: dir,
                            category,
                            start_paused: paused,
                            ..Default::default()
                        },
                    )
                    .await?;
                serde_json::json!({ "id": id })
            }
            Request::Stop { ids } => {
                engine.stop(&resolve_ids(&known(engine), &ids)?).await?;
                serde_json::Value::Null
            }
            Request::Start { ids } => {
                engine.resume(&resolve_ids(&known(engine), &ids)?)?;
                serde_json::Value::Null
            }
            Request::Remove { ids, delete_files } => {
                ensure!(
                    !ids.iter().any(|i| i == "all"),
                    "remove torrents one by one"
                );
                engine
                    .remove(&resolve_ids(&known(engine), &ids)?, delete_files)
                    .await?;
                serde_json::Value::Null
            }
            Request::Shutdown => serde_json::Value::Null,
        })
    }
}

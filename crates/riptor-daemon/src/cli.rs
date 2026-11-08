//! riptor-cli's commands and their help.

use std::path::Path;

use anyhow::{bail, Context};

use crate::Request;

pub enum Parsed {
    Run(Request),
    Help(String),
}

struct Command {
    name: &'static str,
    aliases: &'static [&'static str],
    usage: &'static str,
    about: &'static str,
    /// Flag, then what it does; a flag with a space takes a value.
    options: &'static [(&'static str, &'static str)],
    takes_ids: bool,
}

const IDS: &str = "Ids are info hashes; 6 or more leading characters are enough.";

const COMMANDS: &[Command] = &[
    Command {
        name: "list",
        aliases: &["ls"],
        usage: "riptor-cli list",
        about: "Shows every torrent with its id, state, progress and speeds.",
        options: &[],
        takes_ids: false,
    },
    Command {
        name: "stats",
        aliases: &[],
        usage: "riptor-cli stats",
        about: "Shows the network mode, total speeds and listening port.",
        options: &[],
        takes_ids: false,
    },
    Command {
        name: "add",
        aliases: &[],
        usage: "riptor-cli add <magnet | https link | /path/file.torrent> [--dir FOLDER] [--category NAME] [--paused]",
        about: "Adds a torrent. A relative file path or folder is taken from the current folder.",
        options: &[
            ("--dir FOLDER", "Save it here instead of the default download folder"),
            ("--category NAME", "Put it in this category"),
            ("--paused", "Add it stopped"),
        ],
        takes_ids: false,
    },
    Command {
        name: "stop",
        aliases: &["pause"],
        usage: "riptor-cli stop <id...|all>",
        about: "Stops torrents, or every torrent with all.",
        options: &[],
        takes_ids: true,
    },
    Command {
        name: "start",
        aliases: &["resume"],
        usage: "riptor-cli start <id...|all>",
        about: "Starts torrents, or every torrent with all.",
        options: &[],
        takes_ids: true,
    },
    Command {
        name: "remove",
        aliases: &["rm"],
        usage: "riptor-cli remove <id...> [--delete-files]",
        about: "Removes torrents from Riptor. Their files stay unless you ask otherwise.",
        options: &[("--delete-files", "Delete the downloaded files too")],
        takes_ids: true,
    },
    Command {
        name: "shutdown",
        aliases: &[],
        usage: "riptor-cli shutdown",
        about: "Stops riptord.",
        options: &[],
        takes_ids: false,
    },
];

fn find(name: &str) -> Option<&'static Command> {
    COMMANDS
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name))
}

fn is_help(arg: &str) -> bool {
    arg == "-h" || arg == "--help"
}

pub fn usage() -> String {
    let mut out: Vec<&str> = COMMANDS.iter().map(|c| c.usage).collect();
    out.push("");
    out.push(IDS);
    out.push("Run riptor-cli <command> --help for more about one command.");
    out.join("\n")
}

fn help(c: &Command) -> String {
    let mut out = format!("{}\n\n{}\n", c.usage, c.about);
    if !c.options.is_empty() {
        out.push('\n');
        let width = c.options.iter().map(|(f, _)| f.len()).max().unwrap_or(0);
        for (flag, what) in c.options {
            out.push_str(&format!("  {flag:<width$}  {what}\n"));
        }
    }
    if c.takes_ids {
        out.push_str(&format!("\n{IDS}\n"));
    }
    if !c.aliases.is_empty() {
        out.push_str(&format!("\nAlso: {}\n", c.aliases.join(", ")));
    }
    out.trim_end().to_string()
}

/// `cwd` resolves relative paths, which mean nothing to the daemon.
pub fn parse(args: &[String], cwd: &Path) -> anyhow::Result<Parsed> {
    let Some((name, rest)) = args.split_first() else {
        return Ok(Parsed::Help(usage()));
    };
    if is_help(name) || name == "help" {
        return match rest.first() {
            None => Ok(Parsed::Help(usage())),
            Some(n) => match find(n) {
                Some(c) => Ok(Parsed::Help(help(c))),
                None => bail!("unknown command {n}\n\n{}", usage()),
            },
        };
    }
    let c = find(name).with_context(|| format!("unknown command {name}\n\n{}", usage()))?;
    if rest.iter().any(|a| is_help(a)) {
        return Ok(Parsed::Help(help(c)));
    }

    let mut words = Vec::new();
    let mut flags = Vec::new();
    let mut values = Vec::new();
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        if !arg.starts_with('-') || arg == "-" {
            words.push(arg.clone());
            continue;
        }
        let Some((spec, _)) = c
            .options
            .iter()
            .find(|(f, _)| f.split(' ').next() == Some(arg.as_str()))
        else {
            bail!(
                "{name} has no option {arg}; see riptor-cli {} --help",
                c.name
            );
        };
        if spec.contains(' ') {
            let value = it.next().with_context(|| format!("{arg} needs a value"))?;
            values.push((arg.as_str(), value.clone()));
        } else {
            flags.push(arg.as_str());
        }
    }
    let flag = |f: &str| flags.contains(&f);
    let value = |f: &str| values.iter().find(|(k, _)| *k == f).map(|(_, v)| v.clone());
    if !c.takes_ids && c.name != "add" && !words.is_empty() {
        bail!(
            "{} takes no arguments; see riptor-cli {} --help",
            c.name,
            c.name
        );
    }

    Ok(Parsed::Run(match c.name {
        "list" => Request::List,
        "stats" => Request::Stats,
        "add" => {
            let [source] = words.as_slice() else {
                bail!("add needs one torrent; see riptor-cli add --help");
            };
            let source = if !source.contains(':') && !source.starts_with('/') {
                cwd.join(source).to_string_lossy().into_owned()
            } else {
                source.clone()
            };
            Request::Add {
                source,
                dir: value("--dir").map(|d| cwd.join(d)),
                category: value("--category"),
                paused: flag("--paused"),
            }
        }
        "stop" => Request::Stop { ids: words },
        "start" => Request::Start { ids: words },
        "remove" => Request::Remove {
            ids: words,
            delete_files: flag("--delete-files"),
        },
        "shutdown" => Request::Shutdown,
        _ => unreachable!("every command is handled"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn run(args: &[&str]) -> anyhow::Result<Parsed> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        parse(&args, Path::new("/home/me"))
    }

    fn help_text(args: &[&str]) -> String {
        match run(args).unwrap() {
            Parsed::Help(t) => t,
            Parsed::Run(r) => panic!("{args:?} ran {r:?} instead of showing help"),
        }
    }

    fn request(args: &[&str]) -> Request {
        match run(args).unwrap() {
            Parsed::Run(r) => r,
            Parsed::Help(_) => panic!("{args:?} showed help"),
        }
    }

    fn error(args: &[&str]) -> String {
        match run(args) {
            Err(e) => format!("{e:#}"),
            Ok(_) => panic!("{args:?} was accepted"),
        }
    }

    #[test]
    fn overview_with_no_arguments_or_help() {
        for args in [&[][..], &["help"], &["--help"], &["-h"]] {
            let t = help_text(args);
            assert!(t.starts_with("riptor-cli list\n"), "{args:?}");
            assert!(t.contains("riptor-cli <command> --help"));
            for c in COMMANDS {
                assert!(t.contains(c.usage), "{} missing", c.name);
            }
        }
    }

    #[test]
    fn help_for_one_command_by_any_name() {
        let add = help_text(&["add", "--help"]);
        assert!(add.starts_with("riptor-cli add <magnet"));
        assert!(add.contains("--category NAME  Put it in this category"));
        assert!(!add.contains("riptor-cli list"), "only the one command");
        for args in [
            &["add", "-h"][..],
            &["help", "add"],
            &["--help", "add"],
            &["add", "magnet:?xt=x", "--help"],
        ] {
            assert_eq!(help_text(args), add, "{args:?}");
        }
        // Help wins over ids and aliases find the command.
        let stop = help_text(&["pause", "abcdef12", "--help"]);
        assert!(stop.starts_with("riptor-cli stop"));
        assert!(stop.contains("Also: pause") && stop.contains(IDS));
        assert!(help_text(&["rm", "-h"]).contains("--delete-files"));
        assert!(help_text(&["help", "shutdown"]).contains("Stops riptord."));
    }

    #[test]
    fn commands_build_requests() {
        assert_eq!(request(&["ls"]), Request::List);
        assert_eq!(
            request(&[
                "add",
                "file.torrent",
                "--dir",
                "dl",
                "--category",
                "iso",
                "--paused"
            ]),
            Request::Add {
                source: PathBuf::from("/home/me")
                    .join("file.torrent")
                    .to_string_lossy()
                    .into_owned(),
                dir: Some(PathBuf::from("/home/me").join("dl")),
                category: Some("iso".into()),
                paused: true,
            }
        );
        assert_eq!(
            request(&["add", "--paused", "magnet:?xt=urn:btih:abc"]),
            Request::Add {
                source: "magnet:?xt=urn:btih:abc".into(),
                dir: None,
                category: None,
                paused: true,
            }
        );
        assert_eq!(
            request(&["resume", "all"]),
            Request::Start {
                ids: vec!["all".into()]
            }
        );
        assert_eq!(
            request(&["rm", "abcdef", "--delete-files", "123456"]),
            Request::Remove {
                ids: vec!["abcdef".into(), "123456".into()],
                delete_files: true,
            }
        );
    }

    #[test]
    fn mistakes_say_what_is_wrong() {
        assert!(error(&["nope"]).starts_with("unknown command nope\n\nriptor-cli list"));
        assert!(error(&["help", "nope"]).starts_with("unknown command nope"));
        assert!(error(&["add", "x.torrent", "--dri", "d"]).contains("add has no option --dri"));
        assert!(error(&["stop", "--delete-files", "abcdef"]).contains("stop has no option"));
        assert_eq!(error(&["add", "x.torrent", "--dir"]), "--dir needs a value");
        assert!(error(&["add"]).contains("add needs one torrent"));
        assert!(error(&["add", "a.torrent", "b.torrent"]).contains("add needs one torrent"));
        assert!(error(&["list", "extra"]).contains("list takes no arguments"));
    }
}

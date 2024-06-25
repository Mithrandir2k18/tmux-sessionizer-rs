use path_clean::PathClean;
use rayon::prelude::*;
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct Cli {
    #[structopt(
        short,
        long,
        parse(from_os_str),
        help = "Path to YAML configuration file"
    )]
    config: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct Config {
    search_paths: Vec<Option<String>>,
    nested: Option<bool>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::from_args();
    let config = load_config(args.config);
    let nested = config.nested.unwrap_or(false);

    let search_paths = filter_contained_paths(config.search_paths);

    let repos: Vec<PathBuf> = search_paths
        .par_iter()
        .filter_map(|root| {
            if root.exists() {
                Some(find_git_repos(root, nested))
            } else {
                eprintln!("Path does not exist: {}", root.display());
                None
            }
        })
        .flatten()
        .collect();

    let choices = repos
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>();
    let selected = fzf_select(&choices)?;

    if selected.is_empty() {
        return Ok(());
    }

    let selected_path = Path::new(&selected);
    let selected_name = selected_path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .replace('.', "_");

    if !is_tmux_running() {
        start_tmux_session(&selected_name, &selected_path)?;
    }

    switch_tmux_client(&selected_name, &selected_path)?;

    Ok(())
}

fn load_config(config_path: Option<PathBuf>) -> Config {
    if let Some(path) = config_path {
        let config_content = fs::read_to_string(path).expect("Failed to read configuration file");
        serde_yaml::from_str(&config_content).expect("Failed to parse configuration file")
    } else {
        eprintln!("Configuration file is required.");
        std::process::exit(1);
    }
}

fn filter_contained_paths(paths: Vec<Option<String>>) -> Vec<PathBuf> {
    let mut expanded_cleaned_paths: Vec<PathBuf> = paths
        .into_iter()
        .filter_map(|item| item.map(PathBuf::from))
        .map(|p| shellexpand::tilde(p.to_str().unwrap()).to_string())
        .map(|p| PathBuf::from(p).clean())
        .collect();


    expanded_cleaned_paths.sort();
    expanded_cleaned_paths.dedup();

    let mut result = Vec::new();

    for path in &expanded_cleaned_paths {
        if !expanded_cleaned_paths
            .iter()
            .any(|other| other != path && path.starts_with(other))
        {
            result.push(path.clone());
        }
    }

    result
}

fn find_git_repos(root: &Path, nested: bool) -> Vec<PathBuf> {
    if !root.is_dir() {
        return Vec::new();
    }

    let mut git_repos = Vec::new();

    let entries: Vec<_> = fs::read_dir(root).unwrap().filter_map(Result::ok).collect();
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join(".git").exists() {
            git_repos.push(path.clone());
            if nested {
                git_repos.extend(find_git_repos(&path, nested));
            }
            continue;
        }

        git_repos.extend(find_git_repos(&path, nested));
    }

    git_repos
}

fn fzf_select(choices: &[String]) -> Result<String, Box<dyn std::error::Error>> {
    use std::io::Write;

    let mut child = Command::new("fzf")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    {
        let stdin = child.stdin.as_mut().ok_or("Failed to open stdin")?;
        for choice in choices {
            writeln!(stdin, "{}", choice)?;
        }
    }

    let output = child.wait_with_output()?;
    let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(selected)
}

fn is_tmux_running() -> bool {
    env::var("TMUX").is_ok()
        || Command::new("pgrep")
            .arg("tmux")
            .output()
            .map_or(false, |o| o.status.success())
}

fn start_tmux_session(session_name: &str, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Command::new("tmux")
        .arg("new-session")
        .arg("-s")
        .arg(session_name)
        .arg("-c")
        .arg(path)
        .status()?;
    Ok(())
}

fn switch_tmux_client(session_name: &str, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let has_session = Command::new("tmux")
        .arg("has-session")
        .arg("-t")
        .arg(session_name)
        .output()?
        .status
        .success();

    if !has_session {
        Command::new("tmux")
            .arg("new-session")
            .arg("-ds")
            .arg(session_name)
            .arg("-c")
            .arg(path)
            .status()?;
    }

    Command::new("tmux")
        .arg("switch-client")
        .arg("-t")
        .arg(session_name)
        .status()?;

    Ok(())
}

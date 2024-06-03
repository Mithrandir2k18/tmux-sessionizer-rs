use rayon::prelude::*;
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
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
    #[structopt(short, long, help = "Search for and include nested git repos")]
    nested: bool,
}

#[derive(Debug, Deserialize)]
struct Config {
    search_paths: Vec<Option<String>>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::from_args();
    let nested: bool = args.nested;
    let config: Config = if let Some(config_path) = args.config {
        let config_content = fs::read_to_string(config_path)?;
        match serde_yaml::from_str(&config_content) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("Failed to parse configuration file: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("Configuration file is required.");
        std::process::exit(1);
    };

    let search_paths: Vec<String> = config
        .search_paths
        .into_iter()
        .filter_map(|item| item)
        .collect();

    let repos: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    search_paths.par_iter().for_each(|root| {
        let root_path = shellexpand::tilde(root).to_string();
        let root_path = Path::new(&root_path);

        if root_path.exists() {
            let found_repos = find_git_repos(root_path, nested);
            let mut repos_lock = repos.lock().unwrap();
            repos_lock.extend(found_repos);
        } else {
            eprintln!("Path does not exist: {}", root_path.display());
        }
    });

    let repos = Arc::try_unwrap(repos).unwrap().into_inner().unwrap();
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

    let tmux_running = Command::new("pgrep").arg("tmux").output()?.status.success();

    if env::var("TMUX").is_err() && !tmux_running {
        Command::new("tmux")
            .arg("new-session")
            .arg("-s")
            .arg(&selected_name)
            .arg("-c")
            .arg(selected_path)
            .status()?;
        return Ok(());
    }

    let has_session = Command::new("tmux")
        .arg("has-session")
        .arg("-t")
        .arg(&selected_name)
        .output()?
        .status
        .success();

    if !has_session {
        Command::new("tmux")
            .arg("new-session")
            .arg("-ds")
            .arg(&selected_name)
            .arg("-c")
            .arg(selected_path)
            .status()?;
    }

    Command::new("tmux")
        .arg("switch-client")
        .arg("-t")
        .arg(&selected_name)
        .status()?;

    Ok(())
}

fn find_git_repos(root: &Path, nested: bool) -> Vec<PathBuf> {
    if !root.is_dir() {
        return Vec::new();
    }

    let entries: Vec<_> = fs::read_dir(root).unwrap().filter_map(Result::ok).collect();

    let (repos, dirs): (Vec<_>, Vec<_>) = entries
        .par_iter()
        .filter(|entry| entry.path().is_dir())
        .partition(|entry| entry.path().join(".git").exists());

    let mut git_repos: Vec<_> = repos.par_iter().map(|entry| entry.path()).collect();

    let nested_dirs: Vec<_> = dirs
        .par_iter()
        .flat_map(|entry| find_git_repos(&entry.path(), nested))
        .collect();

    git_repos.extend(nested_dirs);

    if nested {
        let nested_repos: Vec<_> = repos
            .par_iter()
            .flat_map(|entry| find_git_repos(&entry.path(), nested))
            .collect();

        git_repos.extend(nested_repos)
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

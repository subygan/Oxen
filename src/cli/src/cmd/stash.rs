use async_trait::async_trait;
use clap::{Arg, Command};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use liboxen::constants;
use liboxen::core::v_latest::revisions;
use liboxen::core::v_latest::status;
use liboxen::error::OxenError;
use liboxen::model::LocalRepository;
use liboxen::repositories::commits;
use liboxen::util;

use crate::cmd::RunCmd;

pub const STASH_COMMAND: &str = "stash";

#[derive(Debug)]
pub struct StashCmd;

#[async_trait]
impl RunCmd for StashCmd {
    fn name(&self) -> &str {
        STASH_COMMAND
    }

    fn args(&self) -> Command {
        Command::new(STASH_COMMAND)
            .about("Stash local changes")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(
                Command::new("push")
                    .about("Push changes to the stash")
                    .arg(
                        Arg::new("message")
                            .short('m')
                            .long("message")
                            .help("Optional message to describe the stash")
                            .action(clap::ArgAction::Set)
                            .num_args(1),
                    ),
            )
            .subcommand(Command::new("pop").about("Apply the latest stash and remove it"))
            .subcommand(Command::new("apply").about("Apply the latest stash but do not remove it"))
            .subcommand(Command::new("list").about("List all stashes"))
    }

    async fn run(&self, args: &clap::ArgMatches) -> Result<(), OxenError> {
        match args.subcommand() {
            Some(("push", sub_args)) => {
                let message = sub_args.get_one::<String>("message").map(|s| s.as_str());
                let repo = LocalRepository::from_current_dir()?;
                let status = status::status(&repo)?;

                if status.modified_files.is_empty() {
                    println!("No changes to stash.");
                    return Ok(());
                }

                let head_commit = commits::head_commit(&repo)?;

                // Create a unique directory name for the stash
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| OxenError::basic_str(format!("SystemTime Error: {e}")))?
                    .as_millis();
                let stash_name = format!("stash_{}", timestamp);

                let hidden_dir = util::fs::oxen_hidden_dir(&repo.path);
                let stash_base_dir = hidden_dir.join(constants::STASH_DIR);
                let stash_instance_dir = stash_base_dir.join(&stash_name);

                fs::create_dir_all(&stash_instance_dir)?;

                if let Some(msg_content) = message {
                    let message_file_path = stash_instance_dir.join("message.txt");
                    fs::write(message_file_path, msg_content)?;
                }

                println!("Stashing modified files...");
                for path in status.modified_files.iter() {
                    let source_path = repo.path.join(path);
                    let dest_path = stash_instance_dir.join(path);

                    if let Some(parent) = dest_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::copy(&source_path, &dest_path)?;
                    println!("  Stashed: {}", path.display());
                }

                println!("Reverting modified files to HEAD...");
                for path in status.modified_files.iter() {
                    let working_dir_file_path = repo.path.join(path);
                    match revisions::get_version_file_from_commit_id(
                        &repo,
                        &head_commit.id,
                        path,
                    ) {
                        Ok(head_version_path) => {
                            // Ensure parent directory exists for working_dir_file_path just in case
                            if let Some(parent) = working_dir_file_path.parent() {
                                fs::create_dir_all(parent)?;
                            }
                            fs::copy(&head_version_path, &working_dir_file_path)?;
                            println!("  Reverted: {}", path.display());
                        }
                        Err(_) => {
                            // This file is not in the HEAD commit (it's a new file). Reverting means removing it.
                            match fs::remove_file(&working_dir_file_path) {
                                Ok(_) => println!("  Removed new file (reverted): {}", path.display()),
                                Err(remove_err) => {
                                    // Log an error if removal fails, but continue the stash operation.
                                    // It's possible the file was already deleted or is locked.
                                    eprintln!("Warning: could not remove new file {} during stash operation: {}", path.display(), remove_err);
                                }
                            }
                        }
                    }
                }

                // The message is now saved to message.txt, so the specific printout and TODO are removed.
                println!("\nCreated stash: {}", stash_name);
                Ok(())
            }
            Some(("pop", _sub_args)) => {
                let repo = LocalRepository::from_current_dir()?;
                let hidden_dir = util::fs::oxen_hidden_dir(&repo.path);
                let stash_base_dir = hidden_dir.join(constants::STASH_DIR);

                if !stash_base_dir.exists() {
                    println!("No stashes to pop.");
                    return Ok(());
                }

                let mut stash_dirs: Vec<String> = fs::read_dir(&stash_base_dir)?
                    .filter_map(Result::ok)
                    .filter(|entry| {
                        entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                            && entry.file_name().to_string_lossy().starts_with("stash_")
                    })
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect();

                stash_dirs.sort_unstable(); // Sorts alphabetically, which works for timestamp-based names

                if let Some(latest_stash_name) = stash_dirs.last() {
                    let latest_stash_path = stash_base_dir.join(latest_stash_name);
                    let message_file_path = latest_stash_path.join("message.txt");
                    match fs::read_to_string(&message_file_path) {
                        Ok(content) => {
                            println!("Popping stash: {} - {}", latest_stash_name, content.trim());
                        }
                        Err(_) => {
                            println!("Popping stash: {}", latest_stash_name);
                        }
                    }

                    let stashed_files = util::fs::rlist_paths_in_dir(&latest_stash_path);
                    for stashed_file_path in stashed_files.iter() {
                        let relative_path = stashed_file_path.strip_prefix(&latest_stash_path)
                            .map_err(|e| OxenError::basic_str(format!("Error stripping prefix: {}", e)))?;
                        let working_dir_dest_path = repo.path.join(relative_path);

                        if let Some(parent) = working_dir_dest_path.parent() {
                            fs::create_dir_all(parent)?;
                        }
                        fs::copy(stashed_file_path, &working_dir_dest_path)?;
                        println!("  Applied: {}", relative_path.display());
                    }

                    // After successfully copying all files, remove the stash directory
                    fs::remove_dir_all(&latest_stash_path)?;
                    println!("\nApplied and removed stash: {}", latest_stash_name);
                } else {
                    println!("No stashes to pop.");
                }
                Ok(())
            }
            Some(("apply", _sub_args)) => {
                let repo = LocalRepository::from_current_dir()?;
                let hidden_dir = util::fs::oxen_hidden_dir(&repo.path);
                let stash_base_dir = hidden_dir.join(constants::STASH_DIR);

                if !stash_base_dir.exists() {
                    println!("No stashes to apply.");
                    return Ok(());
                }

                let mut stash_dirs: Vec<String> = fs::read_dir(&stash_base_dir)?
                    .filter_map(Result::ok)
                    .filter(|entry| {
                        entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                            && entry.file_name().to_string_lossy().starts_with("stash_")
                    })
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect();

                stash_dirs.sort_unstable(); // Sorts alphabetically, which works for timestamp-based names

                if let Some(latest_stash_name) = stash_dirs.last() {
                    let latest_stash_path = stash_base_dir.join(latest_stash_name);
                    println!("Applying stash: {}", latest_stash_name);

                    let stashed_files = util::fs::rlist_paths_in_dir(&latest_stash_path);
                    for stashed_file_path in stashed_files.iter() {
                        let relative_path = stashed_file_path.strip_prefix(&latest_stash_path)
                            .map_err(|e| OxenError::basic_str(format!("Error stripping prefix: {}", e)))?;
                        let working_dir_dest_path = repo.path.join(relative_path);

                        if let Some(parent) = working_dir_dest_path.parent() {
                            fs::create_dir_all(parent)?;
                        }
                        fs::copy(stashed_file_path, &working_dir_dest_path)?;
                        println!("  Applied: {}", relative_path.display());
                    }
                    // DO NOT remove the stash directory for 'apply'
                    println!("\nApplied stash: {}", latest_stash_name);
                } else {
                    println!("No stashes to apply.");
                }
                Ok(())
            }
            Some(("list", _sub_args)) => {
                let repo = LocalRepository::from_current_dir()?;
                let hidden_dir = util::fs::oxen_hidden_dir(&repo.path);
                let stash_base_dir = hidden_dir.join(constants::STASH_DIR);

                if !stash_base_dir.exists() {
                    println!("No stashes available.");
                    return Ok(());
                }

                let stash_names: Vec<String> = fs::read_dir(&stash_base_dir)?
                    .filter_map(Result::ok)
                    .filter(|entry| {
                        entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                            && entry.file_name().to_string_lossy().starts_with("stash_")
                    })
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect();

                if stash_names.is_empty() {
                    println!("No stashes available.");
                } else {
                    println!("Available stashes:");
                    for stash_name in stash_names {
                        let stash_dir_path = stash_base_dir.join(&stash_name);
                        let message_file_path = stash_dir_path.join("message.txt");
                        match fs::read_to_string(&message_file_path) {
                            Ok(content) => {
                                println!(" - {}: {}", stash_name, content.trim());
                            }
                            Err(_) => {
                                println!(" - {}", stash_name);
                            }
                        }
                    }
                }
                Ok(())
            }
            Some((name, _sub_args)) => Err(OxenError::basic_str(format!(
                "Unknown {} command: {}",
                self.name(),
                name
            ))),
            None => {
                // This case should not be reached due to `subcommand_required(true)`
                // and `arg_required_else_help(true)` in the command definition.
                // However, it's good practice to handle it.
                Err(OxenError::basic_str(format!(
                    "Usage: oxen {} <SUBCOMMAND>",
                    self.name()
                )))
            }
        }
    }
}

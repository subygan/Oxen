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

                let head_commit_file_path = stash_instance_dir.join("head_commit.txt");
                fs::write(head_commit_file_path, head_commit.id.clone())?;

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

                    let mut conflicted_files: Vec<std::path::PathBuf> = Vec::new();
                    let base_commit_id = fs::read_to_string(latest_stash_path.join("head_commit.txt"))
                        .map_err(|e| OxenError::basic_str(format!("Failed to read base_commit_id for stash: {}", e)))?
                        .trim().to_string();

                    let stashed_files = util::fs::rlist_paths_in_dir(&latest_stash_path);
                    for stashed_file_path in stashed_files.iter() {
                        // Skip meta files like head_commit.txt and message.txt
                        if stashed_file_path.file_name().map_or(false, |name| name == "head_commit.txt" || name == "message.txt") {
                            continue;
                        }

                        let relative_path = stashed_file_path.strip_prefix(&latest_stash_path)
                            .map_err(|e| OxenError::basic_str(format!("Error stripping prefix: {}", e)))?;
                        let working_dir_dest_path = repo.path.join(relative_path);
                        let metadata = fs::metadata(&stashed_file_path)?;

                        if metadata.is_dir() {
                            // Ensure directory exists in working dir, no conflict logic for dirs for now
                            fs::create_dir_all(&working_dir_dest_path)?;
                            println!("  Created directory: {}", relative_path.display());
                            continue; // move to next stashed_file_path
                        }

                        // Logic for files
                        if metadata.is_file() {
                            let stashed_content = fs::read(&stashed_file_path)?;
                            let local_content_opt = if working_dir_dest_path.exists() {
                                Some(fs::read(&working_dir_dest_path)?)
                            } else {
                                None
                            };

                            match revisions::get_version_file_from_commit_id(&repo, &base_commit_id, relative_path) {
                                Ok(base_version_actual_path) => {
                                    // Case 2: File existed in base_commit_id
                                    let base_content = fs::read(&base_version_actual_path)?;
                                    let local_content = if working_dir_dest_path.exists() {
                                        std::fs::read(&working_dir_dest_path)?
                                    } else {
                                        Vec::new() // Treat as empty if not existent or deleted
                                    };

                                    let is_local_modified = local_content != base_content;
                                    let is_stashed_modified = stashed_content != base_content;

                                    if is_local_modified && is_stashed_modified {
                                        if local_content != stashed_content {
                                            // True Conflict
                                            println!("Conflict: File {} changed locally and in stash. Keeping local version.", relative_path.display());
                                            conflicted_files.push(relative_path.to_path_buf());
                                            // Add these lines:
                                            println!("  Your local changes for '{}' have been kept.", relative_path.display());
                                            println!("  To view the stashed version, inspect: .oxen/stash/{}/{}", latest_stash_name, relative_path.display());
                                            println!("  To view the base version (from when you stashed), you can use: oxen checkout {} -- '{}'", base_commit_id, relative_path.display());
                                            println!("  Consider using 'oxen stash apply_file {} {}' to apply the stashed version for this file if desired.", latest_stash_name, relative_path.display());
                                            println!("  (Note: 'oxen stash apply_file' and 'oxen stash drop' might be future commands for finer control).");
                                        } else {
                                            // Convergent Edit
                                            println!("Applied convergent edit for file: {}", relative_path.display());
                                            if let Some(parent) = working_dir_dest_path.parent() {
                                                std::fs::create_dir_all(parent)?;
                                            }
                                            std::fs::copy(&stashed_file_path, &working_dir_dest_path)?;
                                        }
                                    } else if is_stashed_modified {
                                        println!("Applied stashed changes to file: {}", relative_path.display());
                                        if let Some(parent) = working_dir_dest_path.parent() {
                                            std::fs::create_dir_all(parent)?;
                                        }
                                        std::fs::copy(&stashed_file_path, &working_dir_dest_path)?;
                                    } else if is_local_modified {
                                        println!("Kept local changes for file: {}", relative_path.display());
                                    } else {
                                        println!("File {} is already consistent.", relative_path.display());
                                    }
                                }
                                Err(_) => {
                                    // Case 1: File was new in the stash (not in base_commit_id)
                                    // This logic remains unchanged from the previous step
                                    if local_content_opt.is_some() {
                                        conflicted_files.push(relative_path.to_path_buf());
                                        println!("Conflict: File {} created locally and in stash.", relative_path.display());
                                    } else {
                                        if let Some(parent) = working_dir_dest_path.parent() {
                                            fs::create_dir_all(parent)?;
                                        }
                                        fs::copy(&stashed_file_path, &working_dir_dest_path)?;
                                        println!("  Applied new file: {}", relative_path.display());
                                    }
                                }
                            }
                        }
                    }

                    if conflicted_files.is_empty() {
                        println!("Successfully popped stash: {}", latest_stash_name);
                        fs::remove_dir_all(&latest_stash_path)?;
                    } else {
                        println!("Stash operation completed with conflicts in the following files:");
                        for path in conflicted_files {
                            println!("  - {}", path.display());
                        }
                        println!("Stash '{}' was not removed due to conflicts.", latest_stash_name);
                    }
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
                    let message_file_path = latest_stash_path.join("message.txt");
                    match fs::read_to_string(&message_file_path) {
                        Ok(content) => {
                            println!("Applying stash: {} - {}", latest_stash_name, content.trim());
                        }
                        Err(_) => {
                            println!("Applying stash: {}", latest_stash_name);
                        }
                    }

                    let mut conflicted_files: Vec<std::path::PathBuf> = Vec::new();
                    let base_commit_id = fs::read_to_string(latest_stash_path.join("head_commit.txt"))
                        .map_err(|e| OxenError::basic_str(format!("Failed to read base_commit_id for stash: {}",e)))?
                        .trim().to_string();

                    let stashed_files = util::fs::rlist_paths_in_dir(&latest_stash_path);
                    for stashed_file_path in stashed_files.iter() {
                         // Skip meta files like head_commit.txt and message.txt
                        if stashed_file_path.file_name().map_or(false, |name| name == "head_commit.txt" || name == "message.txt") {
                            continue;
                        }

                        let relative_path = stashed_file_path.strip_prefix(&latest_stash_path)
                            .map_err(|e| OxenError::basic_str(format!("Error stripping prefix: {}", e)))?;
                        let working_dir_dest_path = repo.path.join(relative_path);
                        let metadata = fs::metadata(&stashed_file_path)?;

                        if metadata.is_dir() {
                            // Ensure directory exists in working dir, no conflict logic for dirs for now
                            fs::create_dir_all(&working_dir_dest_path)?;
                            println!("  Created directory: {}", relative_path.display());
                            continue; // move to next stashed_file_path
                        }

                        // Logic for files
                        if metadata.is_file() {
                            let stashed_content = fs::read(&stashed_file_path)?;
                            let local_content_opt = if working_dir_dest_path.exists() {
                                Some(fs::read(&working_dir_dest_path)?)
                            } else {
                                None
                            };

                            match revisions::get_version_file_from_commit_id(&repo, &base_commit_id, relative_path) {
                                Ok(base_version_actual_path) => {
                                    // Case 2: File existed in base_commit_id
                                    let base_content = fs::read(&base_version_actual_path)?;
                                    let local_content = if working_dir_dest_path.exists() {
                                        std::fs::read(&working_dir_dest_path)?
                                    } else {
                                        Vec::new() // Treat as empty if not existent or deleted
                                    };

                                    let is_local_modified = local_content != base_content;
                                    let is_stashed_modified = stashed_content != base_content;

                                    if is_local_modified && is_stashed_modified {
                                        if local_content != stashed_content {
                                            // True Conflict
                                            println!("Conflict: File {} changed locally and in stash. Keeping local version.", relative_path.display());
                                            conflicted_files.push(relative_path.to_path_buf());
                                            // Add these lines:
                                            println!("  Your local changes for '{}' have been kept.", relative_path.display());
                                            println!("  To view the stashed version, inspect: .oxen/stash/{}/{}", latest_stash_name, relative_path.display());
                                            println!("  To view the base version (from when you stashed), you can use: oxen checkout {} -- '{}'", base_commit_id, relative_path.display());
                                            println!("  Consider using 'oxen stash apply_file {} {}' to apply the stashed version for this file if desired.", latest_stash_name, relative_path.display());
                                            println!("  (Note: 'oxen stash apply_file' and 'oxen stash drop' might be future commands for finer control).");
                                        } else {
                                            // Convergent Edit
                                            println!("Applied convergent edit for file: {}", relative_path.display());
                                            if let Some(parent) = working_dir_dest_path.parent() {
                                                std::fs::create_dir_all(parent)?;
                                            }
                                            std::fs::copy(&stashed_file_path, &working_dir_dest_path)?;
                                        }
                                    } else if is_stashed_modified {
                                        println!("Applied stashed changes to file: {}", relative_path.display());
                                        if let Some(parent) = working_dir_dest_path.parent() {
                                            std::fs::create_dir_all(parent)?;
                                        }
                                        std::fs::copy(&stashed_file_path, &working_dir_dest_path)?;
                                    } else if is_local_modified {
                                        println!("Kept local changes for file: {}", relative_path.display());
                                    } else {
                                        println!("File {} is already consistent.", relative_path.display());
                                    }
                                }
                                Err(_) => {
                                    // Case 1: File was new in the stash (not in base_commit_id)
                                    // This logic remains unchanged from the previous step
                                    if local_content_opt.is_some() {
                                        conflicted_files.push(relative_path.to_path_buf());
                                        println!("Conflict: File {} created locally and in stash.", relative_path.display());
                                    } else {
                                        if let Some(parent) = working_dir_dest_path.parent() {
                                            fs::create_dir_all(parent)?;
                                        }
                                        fs::copy(&stashed_file_path, &working_dir_dest_path)?;
                                        println!("  Applied new file: {}", relative_path.display());
                                    }
                                }
                            }
                        }
                    }

                    if conflicted_files.is_empty() {
                        println!("Successfully applied stash: {}", latest_stash_name);
                    } else {
                        println!("Stash operation completed with conflicts in the following files:");
                        for path in conflicted_files {
                            println!("  - {}", path.display());
                        }
                        // Apply does not remove the stash, so no additional message needed here for conflicts.
                    }
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

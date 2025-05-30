use async_trait::async_trait;
use clap::{Arg, Command};

use liboxen::command;
use liboxen::error::OxenError;
use liboxen::model::{Commit, LocalRepository};

use crate::cmd::RunCmd;
use crate::helpers::check_repo_migration_needed;

pub const NAME: &str = "stash";
pub struct StashCmd;

fn print_stash_save_message(repo: &LocalRepository, stash_commit: &Commit) -> Result<(), OxenError> {
    // Attempt to get the stash name like stash@{0}
    let stashes = command::stash::list(repo)?;
    if let Some(latest_stash) = stashes.first() {
        // Check if the commit_id matches, to be sure it's the one we just created
        if latest_stash.commit_id == stash_commit.id {
            println!(
                "Saved working directory and index state as {}: {}",
                latest_stash.name, // e.g., stash@{0}
                latest_stash
                    .message
                    .lines()
                    .next()
                    .unwrap_or_default()
            );
            return Ok(());
        }
    }
    // Fallback if we can't get the stash@{0} name easily or it doesn't match
    println!(
        "Saved working directory and index state with commit {}: {}",
        stash_commit.id,
        stash_commit.message.lines().next().unwrap_or_default()
    );
    Ok(())
}

#[async_trait]
impl RunCmd for StashCmd {
    fn name(&self) -> &str {
        NAME
    }

    fn args(&self) -> Command {
        Command::new(NAME)
            .about("Stash changes in a dirty working directory away. `oxen stash` defaults to `oxen stash push`.")
            .subcommand_required(false)
            .arg_required_else_help(false)
            .subcommand(
                Command::new("push")
                    .about("Save your local modifications to a new stash entry.")
                    .arg(
                        Arg::new("message")
                            .help("Optional descriptive message for the stash.")
                            .long("message")
                            .short('m')
                            .action(clap::ArgAction::Set),
                    ),
            )
            .subcommand(
                Command::new("pop")
                    .about("Remove a single stashed state from the stash list and apply it on top of the current working tree state.")
                    .arg(
                        Arg::new("STASH_ID")
                            .help("The stash to apply (e.g., stash@{0} or an index like 0). Defaults to the latest stash (stash@{0}).")
                            .index(1) // Positional argument
                            .action(clap::ArgAction::Set),
                    ),
            )
            .subcommand(
                Command::new("apply")
                    .about("Like pop, but do not remove the stashed state from the stash list.")
                    .arg(
                        Arg::new("STASH_ID")
                            .help("The stash to apply (e.g., stash@{0} or an index like 0). Defaults to the latest stash (stash@{0}).")
                            .index(1) // Positional argument
                            .action(clap::ArgAction::Set),
                    ),
            )
            .subcommand(
                Command::new("list")
                    .about("List the stash entries that you currently have.")
            )
            .subcommand(
                Command::new("drop")
                    .about("Remove a single stashed state from the stash list.")
                     .arg(
                        Arg::new("STASH_ID")
                            .help("The stash to drop (e.g., stash@{0} or an index like 0). Defaults to the latest stash (stash@{0}).")
                            .index(1) // Positional argument
                            .action(clap::ArgAction::Set),
                    ),
            )
            .subcommand(
                Command::new("clear")
                    .about("Remove all stashed entries.")
            )
    }

    async fn run(&self, args: &clap::ArgMatches) -> Result<(), OxenError> {
        let repo = LocalRepository::from_current_dir()?;
        check_repo_migration_needed(&repo)?;

        match args.subcommand() {
            Some(("push", sub_args)) => {
                let message = sub_args.get_one::<String>("message").map(|s| s.as_str());
                match command::stash::save(&repo, message)? {
                    Some(stash_commit) => {
                        print_stash_save_message(&repo, &stash_commit)?;
                    }
                    None => {
                        println!("No local changes to save.");
                    }
                }
            }
            Some(("pop", sub_args)) => {
                let stash_id = sub_args.get_one::<String>("STASH_ID").map(|s| s.as_str());
                command::stash::pop(&repo, stash_id)?;
                let id_msg = stash_id.map_or_else(|| "Latest stash (stash@{0})".to_string(), |id| format!("Stash '{}'", id));
                println!("{} applied and removed.", id_msg);
                println!("You may need to resolve conflicts if any.");
            }
            Some(("apply", sub_args)) => {
                let stash_id = sub_args.get_one::<String>("STASH_ID").map(|s| s.as_str());
                command::stash::apply(&repo, stash_id)?;
                let id_msg = stash_id.map_or_else(|| "Latest stash (stash@{0})".to_string(), |id| format!("Stash '{}'", id));
                println!("{} applied.", id_msg);
                println!("You may need to resolve conflicts if any.");
            }
            Some(("list", _sub_args)) => {
                let stashes = command::stash::list(&repo)?;
                if stashes.is_empty() {
                    println!("No stashes to list.");
                } else {
                    for stash in stashes.iter() {
                        println!("{}: {}", stash.name, stash.message.lines().next().unwrap_or_default());
                    }
                }
            }
            Some(("drop", sub_args)) => {
                let stash_id = sub_args.get_one::<String>("STASH_ID").map(|s| s.as_str());
                command::stash::drop(&repo, stash_id)?;
                let id_msg = stash_id.map_or_else(|| "Latest stash (stash@{0})".to_string(), |id| format!("Stash '{}'", id));
                println!("Dropped {}.", id_msg);
            }
            Some(("clear", _sub_args)) => {
                command::stash::clear(&repo)?;
                println!("All stashes have been cleared.");
            }
            None => { // Default to `oxen stash push`
                log::debug!("No subcommand given, defaulting to `oxen stash push`");
                match command::stash::save(&repo, None)? {
                    Some(stash_commit) => {
                        print_stash_save_message(&repo, &stash_commit)?;
                    }
                    None => {
                        println!("No local changes to save.");
                    }
                }
            }
            _ => {
                // This case should ideally not be reached if subcommands are defined correctly,
                // as clap would error first for unrecognized subcommands.
                eprintln!("Error: Unknown stash subcommand. Please use `oxen stash --help` for more information.");
                return Err(OxenError::basic_str("Unknown stash subcommand."));
            }
        }
        Ok(())
    }
}
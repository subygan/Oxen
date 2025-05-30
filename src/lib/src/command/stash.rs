use std::str::FromStr;

use crate::constants::OXEN_STASH_DIR;
use crate::core::index::{CommitWriter, IndexReader, Merger, RefWriter, Stager};
use crate::error::OxenError;
use crate::model::{Commit, LocalRepository};
use crate::{api, command, repositories, util};

/// Represents an entry in the stash list
#[derive(Debug, Clone)]
pub struct StashEntry {
    /// Name like stash@{0}, stash@{1}, etc.
    pub name: String,
    /// The ref name, e.g., refs/stashes/0
    pub ref_name: String,
    /// The commit object for this stash
    pub commit: Commit,
}

fn stash_ref_name(idx: usize) -> String {
    format!("{}/{}", OXEN_STASH_DIR, idx)
}

fn get_stash_commit_message(
    repo: &LocalRepository,
    user_message: Option<&str>,
) -> Result<String, OxenError> {
    let head_commit = repositories::commits::head_commit(repo)?;
    let current_branch = repositories::branches::current_branch(repo)?
        .ok_or(OxenError::must_be_on_branch_to_stash())?;

    let mut base_message = format!(
        "WIP on {}: {} {}",
        current_branch.name,
        head_commit.id_prefix(),
        head_commit.message.lines().next().unwrap_or_default()
    );

    if let Some(msg) = user_message {
        base_message.push_str(&format!("\n\n{}", msg));
    }
    Ok(base_message)
}

/// Saves the current state of the working directory and index to a new stash.
/// Returns the Commit of the stash if created, or None if no changes to stash.
pub fn save(repo: &LocalRepository, message: Option<&str>) -> Result<Option<Commit>, OxenError> {
    let status = repositories::status(repo)?;
    if status.is_clean() {
        log::debug!("No local changes to save to stash.");
        return Ok(None);
    }

    let head_commit = repositories::commits::head_commit(repo)?;
    let user = api::local::auth::get_user_from_config(&repo.path)?;

    // 1. Create the stash commit
    // This commit will have the current HEAD as its parent.
    // Its tree will represent the state of the working directory (staged + unstaged + untracked).

    let commit_writer = CommitWriter::new(repo)?;

    // Create a temporary index to build the stash commit's tree
    // This index will represent the full working directory state.
    let mut temp_index_reader = IndexReader::new_from_head(repo)?;

    // Add all modified files from HEAD
    for path in status.modified_files.iter() {
        let absolute_path = repo.path.join(path);
        temp_index_reader.add_file(&absolute_path, repo)?;
    }
    // Add all added files (already in index, but ensure they are part of this commit's view)
    for (path, _entry) in status.staged_files.added_files.iter() {
        let absolute_path = repo.path.join(path);
        temp_index_reader.add_file(&absolute_path, repo)?;
    }
    // Add all untracked files
    for path in status.untracked_files.iter() {
        let absolute_path = repo.path.join(path);
        temp_index_reader.add_file(&absolute_path, repo)?;
    }

    let stash_message = get_stash_commit_message(repo, message)?;
    let parents = vec![head_commit.id.clone()]; // Stash commit is based on current HEAD

    // Commit using the temporary index state
    let stash_commit =
        commit_writer.commit_index(&mut temp_index_reader, &user, &stash_message, parents)?;
    log::debug!(
        "Created stash commit {} with message: {}",
        stash_commit.id,
        stash_commit.message
    );

    // 2. Store the stash ref
    // Newest stash is stash@{0} (refs/stashes/0).
    // We need to shift existing stash refs: refs/stashes/i -> refs/stashes/{i+1}
    let existing_stashes = list_stashes_raw(repo)?;
    let ref_writer = RefWriter::new(repo)?;

    for i in (0..existing_stashes.len()).rev() {
        let old_ref = stash_ref_name(i);
        // The commit_id should be from existing_stashes to avoid re-reading
        let commit_id = &existing_stashes[i].1.id;
        let new_ref = stash_ref_name(i + 1);
        ref_writer.create_ref(&new_ref, commit_id)?;
        log::debug!("Moved stash {} -> {}", old_ref, new_ref);
    }
    // Create the new stash ref for stash@{0}
    ref_writer.create_ref(&stash_ref_name(0), &stash_commit.id)?;
    log::debug!(
        "Saved new stash as {} -> {}",
        stash_ref_name(0),
        stash_commit.id
    );

    // 3. Clean the working directory by resetting to HEAD
    log::debug!(
        "Cleaning working directory by resetting to HEAD {}",
        head_commit.id
    );
    command::reset_hard(repo, &head_commit.id)?;

    Ok(Some(stash_commit))
}

fn list_stashes_raw(repo: &LocalRepository) -> Result<Vec<(String, Commit)>, OxenError> {
    let mut stashes = Vec::new();
    let commit_reader = api::local::commits::commit_reader(repo)?;
    let mut i = 0;
    loop {
        let ref_name = stash_ref_name(i);
        match api::local::refs::get_commit_id_for_ref(repo, &ref_name) {
            Ok(Some(commit_id)) => {
                let commit = commit_reader
                    .get_commit_by_id(&commit_id)?
                    .ok_or(OxenError::commit_id_does_not_exist(commit_id.clone()))?;
                stashes.push((ref_name, commit));
                i += 1;
            }
            Ok(None) => break, // No more stashes
            Err(_) => {
                // Could be a file not found error if the ref doesn't exist, which is normal for termination
                break;
            }
        }
    }
    Ok(stashes)
}

/// Lists all stash entries. Stash@{0} is the most recent.
pub fn list(repo: &LocalRepository) -> Result<Vec<StashEntry>, OxenError> {
    let raw_stashes = list_stashes_raw(repo)?;
    let mut entries = Vec::new();
    for (idx, (ref_name, commit)) in raw_stashes.into_iter().enumerate() {
        entries.push(StashEntry {
            name: format!("stash@{{{}}}", idx),
            ref_name,
            commit,
        });
    }
    Ok(entries)
}

fn resolve_stash_id_to_entry(
    repo: &LocalRepository,
    stash_id_str: Option<&str>,
) -> Result<StashEntry, OxenError> {
    let stashes = list(repo)?;
    if stashes.is_empty() {
        return Err(OxenError::no_stashes_found());
    }

    match stash_id_str {
        None => stashes
            .into_iter()
            .next()
            .ok_or(OxenError::no_stashes_found()), // Default to stash@{0}
        Some(id_str) => {
            // Try parsing as stash@{N} or N
            let num_id_str = id_str
                .trim_start_matches("stash@")
                .trim_matches(|c| c == '{' || c == '}');
            if let Ok(idx) = usize::from_str(num_id_str) {
                return stashes
                    .into_iter()
                    .nth(idx)
                    .ok_or(OxenError::stash_id_not_found(id_str.to_string()));
            }
            // Try as commit ID prefix
            for stash_entry in stashes {
                if stash_entry.commit.id.starts_with(id_str) || stash_entry.ref_name == id_str {
                    return Ok(stash_entry);
                }
            }
            Err(OxenError::stash_id_not_found(id_str.to_string()))
        }
    }
}

/// Applies a stash entry to the working directory. Does not remove the stash.
pub fn apply(repo: &LocalRepository, stash_id: Option<&str>) -> Result<(), OxenError> {
    let stash_entry = resolve_stash_id_to_entry(repo, stash_id)?;
    let stash_commit = &stash_entry.commit;
    log::debug!("Applying stash: {} ({})", stash_entry.name, stash_commit.id);

    let base_head_id = stash_commit.parent_ids.get(0).ok_or_else(|| {
        OxenError::corrupt_stash_commit(
            stash_commit.id.clone(),
            "Missing base HEAD parent".to_string(),
        )
    })?;
    let base_head_commit = repositories::commits::get_by_id(repo, base_head_id)?
        .ok_or_else(|| OxenError::commit_id_does_not_exist(base_head_id.clone()))?;

    let current_head_commit = repositories::commits::head_commit(repo)?;

    let mut merger = Merger::new(repo)?;
    let has_conflicts =
        merger.merge_commits(&current_head_commit, stash_commit, &base_head_commit)?;

    if has_conflicts {
        command::checkout::checkout_index(repo, merger.conflicted_paths(), merger.merged_paths())?;
        println!(
            "Applied stash {} with conflicts. Please resolve them.",
            stash_entry.name
        );
        return Err(OxenError::merge_conflict_err(
            "Conflicts encountered while applying stash.".to_string(),
        ));
    } else {
        command::checkout::checkout_index(repo, Vec::new(), merger.merged_paths())?;
        println!("Applied stash: {}", stash_entry.name);
    }

    Ok(())
}

/// Removes a stash entry from the list and applies it to the working directory.
pub fn pop(repo: &LocalRepository, stash_id: Option<&str>) -> Result<(), OxenError> {
    let stash_to_pop = resolve_stash_id_to_entry(repo, stash_id)?;
    let ref_to_drop = stash_to_pop.ref_name.clone();
    let stash_name_for_msg = stash_to_pop.name.clone();

    log::debug!("Popping stash: {}", stash_name_for_msg);
    apply(repo, Some(&ref_to_drop))?; // Apply using the specific ref_name

    log::debug!("Apply successful, now dropping stash ref: {}", ref_to_drop);
    // Drop the specific stash by its resolved ref_name, which implies its index.
    // We can re-resolve to get the index or parse from ref_name.
    let idx_to_drop: usize = ref_to_drop.split('/').last().unwrap().parse().unwrap();
    drop_by_index(repo, idx_to_drop, &stash_name_for_msg)?;

    Ok(())
}

fn drop_by_index(repo: &LocalRepository, k: usize, name_for_msg: &str) -> Result<(), OxenError> {
    let ref_writer = RefWriter::new(repo)?;
    ref_writer.delete_ref(&stash_ref_name(k))?;
    log::debug!("Deleted stash ref: {}", stash_ref_name(k));

    // Shift subsequent stashes
    let mut i = k + 1;
    loop {
        let old_ref = stash_ref_name(i);
        let new_ref = stash_ref_name(i - 1);
        match api::local::refs::get_commit_id_for_ref(repo, &old_ref) {
            Ok(Some(commit_id)) => {
                ref_writer.create_ref(&new_ref, &commit_id)?;
                ref_writer.delete_ref(&old_ref)?;
                log::debug!("Shifted stash {} -> {}", old_ref, new_ref);
            }
            Ok(None) => break, // No more stashes to shift
            Err(_) => break,   // Error, stop shifting
        }
        i += 1;
    }
    println!("Dropped stash {}.", name_for_msg);
    Ok(())
}

/// Removes a single stash entry from the stash list.
pub fn drop(repo: &LocalRepository, stash_id: Option<&str>) -> Result<(), OxenError> {
    let stash_to_drop = resolve_stash_id_to_entry(repo, stash_id)?;
    let ref_to_drop = stash_to_drop.ref_name;
    let stash_name_for_msg = stash_to_drop.name;

    log::debug!("Dropping stash: {}", stash_name_for_msg);
    let idx_to_drop: usize = ref_to_drop.split('/').last().unwrap().parse().unwrap();
    drop_by_index(repo, idx_to_drop, &stash_name_for_msg)
}

/// Removes all stash entries.
pub fn clear(repo: &LocalRepository) -> Result<(), OxenError> {
    log::debug!("Clearing all stashes.");
    let stashes = list_stashes_raw(repo)?;
    if stashes.is_empty() {
        println!("No stashes to clear.");
        return Ok(());
    }
    let ref_writer = RefWriter::new(repo)?;
    for (idx, _) in stashes.iter().enumerate() {
        let ref_name = stash_ref_name(idx);
        match ref_writer.delete_ref(&ref_name) {
            Ok(_) => log::debug!("Deleted stash ref: {}", ref_name),
            Err(e) => log::warn!("Could not delete stash ref {}: {:?}", ref_name, e),
        }
    }
    println!("All stashes cleared.");
    Ok(())
}
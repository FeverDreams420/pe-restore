//! Finds memory card 1 of the PC port and writes a save into one of its slots.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pe_save_editor::crc::SAVE_BLOCK_SIZE;
use pe_save_editor::save::PeSaveSlot;

const FILE_PREFIX: &str = "BASLUS-00662000000";
const BACKUP_DIR: &str = "pe-restore-backups";

/// Card 1 folder inside a game folder: `memory-cards\bu00` when there is one,
/// else `bu00` next to the game's exe (created on the first write).
pub fn card1_in(game_dir: &Path) -> PathBuf {
    let cards = game_dir.join("memory-cards").join("bu00");
    if cards.is_dir() { cards } else { game_dir.join("bu00") }
}

/// The settings-folder card 1 of newer port builds (Host_CardPath in main.c),
/// when that build has already imported the `bu00` next to its exe: from then
/// on the game reads this card, not the one in the game folder.
pub fn settings_card1() -> Option<PathBuf> {
    let cards = config_dir().ok()?.join("memory-cards");
    cards.join(".legacy-imported").is_file().then(|| cards.join("bu00"))
}

fn config_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("PE_CONFIG_HOME").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    // SDL_GetPrefPath("", "Parasite Eve") on Windows and macOS, XDG on Linux.
    let (var, sub) = if cfg!(windows) {
        ("APPDATA", "Parasite Eve")
    } else if cfg!(target_os = "macos") {
        ("HOME", "Library/Application Support/Parasite Eve")
    } else if std::env::var_os("XDG_CONFIG_HOME").is_some_and(|d| !d.is_empty()) {
        ("XDG_CONFIG_HOME", "parasite-eve")
    } else {
        ("HOME", ".config/parasite-eve")
    };
    std::env::var_os(var)
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join(sub))
        .ok_or_else(|| format!("can't find the game's settings folder (%{var}% is not set)."))
}

/// Save files in `dir` that belong to slot `slot_index` (normal or Clear Data).
pub fn slot_files(dir: &Path, slot_index: usize) -> Vec<PathBuf> {
    let letter = (b'A' + slot_index as u8) as char;
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.len() == 20
                && name.starts_with(FILE_PREFIX)
                && matches!(name.as_bytes()[18], b'0' | b'1')
                && name.ends_with(letter)
                && e.path().is_file()
        })
        .map(|e| e.path())
        .collect();
    files.sort();
    files
}

/// Card title of an existing save file, for the overwrite question.
pub fn describe(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    match fs::read(path).ok().and_then(|b| PeSaveSlot::from_bytes(&b, 0, name.clone()).ok()) {
        Some(slot) => format!("{name} \"{}\"", slot.header_title),
        None => format!("{name} (can't read it)"),
    }
}

/// What `write_slot` did.
pub struct Written {
    pub path: PathBuf,
    pub backups: Vec<PathBuf>,
}

/// Writes `bytes` as `file_name` in `dir`. Files already in that slot are
/// moved to `<dir>/../pe-restore-backups` first, never deleted.
pub fn write_slot(dir: &Path, slot_index: usize, file_name: &str, bytes: &[u8]) -> Result<Written, String> {
    if bytes.len() != SAVE_BLOCK_SIZE {
        return Err(format!("the save is {} bytes, expected {SAVE_BLOCK_SIZE}.", bytes.len()));
    }
    fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let target = dir.join(file_name);

    let old = slot_files(dir, slot_index);
    let mut backups = Vec::new();
    if !old.is_empty() {
        let backup_dir = dir.parent().unwrap_or(dir).join(BACKUP_DIR);
        fs::create_dir_all(&backup_dir).map_err(|e| format!("can't create {}: {e}", backup_dir.display()))?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        for file in &old {
            let name = file.file_name().unwrap().to_string_lossy();
            let mut backup = backup_dir.join(format!("{name}.{stamp}"));
            for n in 1.. {
                if !backup.exists() {
                    break;
                }
                backup = backup_dir.join(format!("{name}.{stamp}-{n}"));
            }
            if let Err(e) = fs::rename(file, &backup) {
                restore(&old, &backups);
                return Err(format!("can't move {} to {}: {e}", file.display(), backup.display()));
            }
            backups.push(backup);
        }
    }

    let temporary = dir.join(format!("{file_name}.pe-restore.tmp"));
    let result = (|| {
        let mut out = fs::File::create_new(&temporary)?;
        out.write_all(bytes)?;
        out.sync_all()?;
        drop(out);
        fs::rename(&temporary, &target)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&temporary);
        restore(&old, &backups);
        return Err(format!("can't write {}: {e}", target.display()));
    }

    match fs::read(&target) {
        Ok(back) if back == bytes => Ok(Written { path: target, backups }),
        _ => Err(format!("{} doesn't read back the same.", target.display())),
    }
}

/// Puts moved files back after a failed write.
fn restore(old: &[PathBuf], backups: &[PathBuf]) {
    for (file, backup) in old.iter().zip(backups) {
        let _ = fs::rename(backup, file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_card() -> PathBuf {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir()
            .join(format!("pe-restore-test-{}-{stamp}", std::process::id()))
            .join("memory-cards")
            .join("bu00");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn writes_into_an_empty_slot() {
        let dir = temp_card();
        let bytes = vec![7u8; SAVE_BLOCK_SIZE];
        let w = write_slot(&dir, 0, "BASLUS-006620000000A", &bytes).unwrap();
        assert_eq!(fs::read(&w.path).unwrap(), bytes);
        assert!(w.backups.is_empty());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        cleanup(&dir);
    }

    #[test]
    fn moves_old_slot_files_to_backups_and_leaves_others() {
        let dir = temp_card();
        fs::write(dir.join("BASLUS-006620000000A"), [1u8; SAVE_BLOCK_SIZE]).unwrap();
        fs::write(dir.join("BASLUS-006620000001A"), [2u8; SAVE_BLOCK_SIZE]).unwrap();
        fs::write(dir.join("BASLUS-006620000000B"), [3u8; SAVE_BLOCK_SIZE]).unwrap();
        assert_eq!(slot_files(&dir, 0).len(), 2);

        let bytes = vec![9u8; SAVE_BLOCK_SIZE];
        let w = write_slot(&dir, 0, "BASLUS-006620000000A", &bytes).unwrap();
        assert_eq!(fs::read(&w.path).unwrap(), bytes);
        assert_eq!(w.backups.len(), 2);
        let kept: Vec<Vec<u8>> = w.backups.iter().map(|b| fs::read(b).unwrap()).collect();
        assert!(kept.contains(&vec![1u8; SAVE_BLOCK_SIZE]));
        assert!(kept.contains(&vec![2u8; SAVE_BLOCK_SIZE]));
        assert!(!dir.join("BASLUS-006620000001A").exists());
        assert_eq!(fs::read(dir.join("BASLUS-006620000000B")).unwrap(), vec![3u8; SAVE_BLOCK_SIZE]);
        cleanup(&dir);
    }

    #[test]
    fn two_writes_in_a_row_keep_both_backups() {
        let dir = temp_card();
        fs::write(dir.join("BASLUS-006620000000A"), [1u8; SAVE_BLOCK_SIZE]).unwrap();
        write_slot(&dir, 0, "BASLUS-006620000000A", &[2u8; SAVE_BLOCK_SIZE]).unwrap();
        let w = write_slot(&dir, 0, "BASLUS-006620000000A", &[3u8; SAVE_BLOCK_SIZE]).unwrap();
        let backups = fs::read_dir(w.backups[0].parent().unwrap()).unwrap().count();
        assert_eq!(backups, 2);
        cleanup(&dir);
    }

    #[test]
    fn card_is_in_the_game_folder() {
        let bu00 = temp_card();
        let game = bu00.parent().unwrap().parent().unwrap().to_path_buf();
        assert_eq!(card1_in(&game), game.join("memory-cards").join("bu00"));
        fs::remove_dir_all(game.join("memory-cards")).unwrap();
        assert_eq!(card1_in(&game), game.join("bu00"));
        let _ = fs::remove_dir_all(&game);
    }

    #[test]
    fn refuses_wrong_size() {
        let dir = temp_card();
        assert!(write_slot(&dir, 0, "BASLUS-006620000000A", &[0u8; 10]).is_err());
        cleanup(&dir);
    }
}

//! Reads a crash-*.txt report from the PC port.

use pe_save_editor::crc::{SAVE_BLOCK_SIZE, SAVE_CRC_OFFSET};
use pe_save_editor::save::RT_OFFSET;

use crate::mods::NamedRecord;

const SAVE_DATA_HEADER: &str = "== Save data (save file bytes from 0x0100) ==";
const SAVE_TAIL_HEADER: &str = "== Save tail (save file bytes from 0x1C90) ==";
const OLD_STORY_HEADER: &str = "== Story progress";
/// The Mods section's list of the snapshot's records (port builds with the host's records).
const MOD_RECORDS_LINE: &str = "Mods' save data (the room snapshot): ";

/// Number of save file bytes a report holds: 0x0100..0x1C8B (everything but
/// the card header/icon in front and the CRC at the end).
pub const SAVE_DATA_LEN: usize = SAVE_CRC_OFFSET - RT_OFFSET;

/// The save's free tail after the CRC, where mods keep their per-save data
/// (Junk, Catalog, key item pocket...). It is copied as it is; `mods` reads
/// it only to show what it holds.
pub const SAVE_TAIL_OFFSET: usize = SAVE_CRC_OFFSET + 4;
pub const SAVE_TAIL_LEN: usize = SAVE_BLOCK_SIZE - SAVE_TAIL_OFFSET;

/// The report's Save tail section (builds with mods only).
#[derive(Debug, PartialEq)]
pub enum SaveTail {
    /// File bytes 0x1C90..0x1FFF at the crash.
    Data(Vec<u8>),
    /// The section is there but empty: mod code was running at the crash.
    NotTaken,
    /// No section: a build without mods, or one older than the section.
    Missing,
}

/// What the tool needs from a crash report.
#[derive(Debug)]
pub struct CrashReport {
    /// "Game: room N".
    pub room: i16,
    /// "story day S" from the Story line; -1 means no game was loaded.
    pub story_day: i32,
    /// Save file bytes 0x0100..0x1C8B, in order.
    pub save_data: Vec<u8>,
    pub save_tail: SaveTail,
    /// The Mods section says "All mods on".
    pub mods_on: bool,
    /// The records the Mods section names: every mod's, with its owner and state at the crash.
    pub mod_records: Vec<NamedRecord>,
}

pub fn parse(text: &str) -> Result<CrashReport, String> {
    let room = find_room(text)?;

    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim_end() == SAVE_DATA_HEADER) else {
        if lines.iter().any(|l| l.starts_with(OLD_STORY_HEADER)) {
            return Err("this report was made by an older build: it only has the story \
                        progress, not the full save data. Make a new report with a newer build."
                .into());
        }
        return Err("this report has no \"== Save data ==\" section.".into());
    };

    let story_day = find_story_day(text)?;
    if story_day < 0 {
        return Err("the game crashed before a save was loaded (title screen), \
                    so there is no story progress to restore."
            .into());
    }

    let save_data = read_hex(&lines[start + 1..], RT_OFFSET, SAVE_DATA_LEN, "save data")?;
    let save_tail = match lines.iter().position(|l| l.trim_end() == SAVE_TAIL_HEADER) {
        None => SaveTail::Missing,
        Some(i) if lines.get(i + 1).is_some_and(|l| l.starts_with("Not available")) => SaveTail::NotTaken,
        Some(i) => SaveTail::Data(read_hex(&lines[i + 1..], SAVE_TAIL_OFFSET, SAVE_TAIL_LEN, "save tail")?),
    };
    let mods_on = lines.iter().any(|l| l.starts_with("Mod API ") && l.contains(", All mods on"));

    // Save_BuildHeader puts the current room in the save, so it must match.
    let saved_room = i16::from_le_bytes([save_data[0x5E], save_data[0x5F]]);
    if saved_room != room {
        return Err(format!("the save data says room {saved_room} but the Game line says room {room}."));
    }

    let mod_records = read_mod_records(&lines);

    Ok(CrashReport { room, story_day, save_data, save_tail, mods_on, mod_records })
}

/// "  4EAF78D1 pe_qol.keys (pe_qol) v1, 68 bytes, live" lines after the Mods section's
/// records line. A name the port didn't know reads "(unknown)" and is left out.
fn read_mod_records(lines: &[&str]) -> Vec<NamedRecord> {
    let Some(start) = lines.iter().position(|l| l.starts_with(MOD_RECORDS_LINE)) else { return Vec::new() };
    let mut out = Vec::new();
    for line in &lines[start + 1..] {
        let Some(rest) = line.strip_prefix("  ") else { break };
        let Some((hex, rest)) = rest.split_once(' ') else { continue };
        let Some(id) = u32::from_str_radix(hex, 16).ok().filter(|_| hex.len() == 8) else { continue };
        let Some((head, state)) = rest.split_once(" bytes, ") else { continue };
        let Some((head, _version)) = head.rsplit_once(" v") else { continue };
        let (name, owner) = match head.split_once(" (") {
            Some((name, owner)) => (name, owner.strip_suffix(')').map(str::to_string)),
            None => (head, None),
        };
        if name != "(unknown)" {
            out.push(NamedRecord { id, name: name.into(), owner, state: state.trim().into() });
        }
    }
    out
}

fn find_room(text: &str) -> Result<i16, String> {
    let line = text
        .lines()
        .find_map(|l| l.strip_prefix("Game: room "))
        .ok_or("this doesn't look like a crash report (no \"Game: room\" line).")?;
    leading_int(line).ok_or_else(|| format!("can't read the room number in \"Game: room {line}\"."))
}

fn find_story_day(text: &str) -> Result<i32, String> {
    let line = text.lines().find(|l| l.starts_with("Story: ")).ok_or("this report has no \"Story:\" line.")?;
    let after = line
        .split_once("story day ")
        .map(|(_, rest)| rest)
        .ok_or_else(|| format!("can't find the story day in \"{line}\"."))?;
    leading_int(after).ok_or_else(|| format!("can't read the story day in \"{line}\"."))
}

/// The number at the start of `text`, e.g. 96 from "96, scene token ...".
fn leading_int<T: std::str::FromStr>(text: &str) -> Option<T> {
    let end = text.find(|c: char| !(c.is_ascii_digit() || c == '-')).unwrap_or(text.len());
    text[..end].parse().ok()
}

/// Reads "OOOO: xx xx .." lines from file offset `start` up to the next blank
/// line or section; `what` names the section in errors.
fn read_hex(lines: &[&str], start: usize, len: usize, what: &str) -> Result<Vec<u8>, String> {
    let mut data = Vec::with_capacity(len);
    for line in lines {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with("==") {
            break;
        }
        let (offset, bytes) = line.split_once(": ").ok_or_else(|| format!("bad line in the {what}: \"{line}\"."))?;
        let offset = usize::from_str_radix(offset, 16).map_err(|_| format!("bad offset in the {what}: \"{line}\"."))?;
        let expected = start + data.len();
        if offset != expected {
            return Err(format!("the {what} jumps from 0x{expected:04X} to 0x{offset:04X} (lines missing)."));
        }
        for byte in bytes.split_whitespace() {
            let value = u8::from_str_radix(byte, 16)
                .ok()
                .filter(|_| byte.len() == 2)
                .ok_or_else(|| format!("bad byte \"{byte}\" in the {what} line {offset:04X}."))?;
            data.push(value);
        }
    }
    if data.len() != len {
        return Err(format!(
            "the {what} has {} bytes, expected {len} (0x{start:04X}..0x{:04X}).",
            data.len(),
            start + len - 1
        ));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A report shaped like the real ones, with `data` written 32 bytes a line.
    fn report(room: i16, story_day: i32, data: &[u8]) -> String {
        let mut text = format!(
            "== What happened ==\n\
             Game: room {room}, scene token 0xA8009348 (current 0xA8009348), disc 1\n\
             Story: day 3 on the save card (progress 328), story day {story_day} (next 4), day flags 0x00\n\
             \n\
             {SAVE_DATA_HEADER}\n"
        );
        text += &hex_lines(RT_OFFSET, data);
        text + "\n== Log of this run ==\nlog line\n"
    }

    fn hex_lines(start: usize, data: &[u8]) -> String {
        let mut text = String::new();
        for (i, chunk) in data.chunks(32).enumerate() {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
            text += &format!("{:04X}: {}\n", start + i * 32, hex.join(" "));
        }
        text
    }

    /// The same report from a mods build: a Mods section, and a Save tail
    /// section with `tail`, or one saying it was not taken.
    fn mods_report(data: &[u8], tail: Option<&[u8]>) -> String {
        let text = report(96, 4, data).replace(
            &format!("\n{SAVE_DATA_HEADER}"),
            &format!("== Mods ==\nMod API 1, mod host started, All mods on\n\n{SAVE_DATA_HEADER}"),
        );
        let section = match tail {
            Some(tail) => hex_lines(SAVE_TAIL_OFFSET, tail),
            None => "Not available: mod code was running at the crash, so the mods' save data was not taken.\n".into(),
        };
        text.replace("\n== Log of this run ==", &format!("\n{SAVE_TAIL_HEADER}\n{section}\n== Log of this run =="))
    }

    fn tail() -> Vec<u8> {
        (0..SAVE_TAIL_LEN).map(|i| (i * 7) as u8).collect()
    }

    fn save_data(room: i16) -> Vec<u8> {
        let mut data: Vec<u8> = (0..SAVE_DATA_LEN).map(|i| i as u8).collect();
        data[0x5E..0x60].copy_from_slice(&room.to_le_bytes());
        data
    }

    #[test]
    fn reads_room_story_day_and_save_data() {
        let data = save_data(96);
        let parsed = parse(&report(96, 4, &data)).unwrap();
        assert_eq!(parsed.room, 96);
        assert_eq!(parsed.story_day, 4);
        assert_eq!(parsed.save_data, data);
    }

    #[test]
    fn reports_without_mods_have_no_tail() {
        let parsed = parse(&report(96, 4, &save_data(96))).unwrap();
        assert_eq!(parsed.save_tail, SaveTail::Missing);
        assert!(!parsed.mods_on);
    }

    #[test]
    fn reads_the_save_tail() {
        let parsed = parse(&mods_report(&save_data(96), Some(&tail()))).unwrap();
        assert_eq!(parsed.save_tail, SaveTail::Data(tail()));
        assert!(parsed.mods_on);
        assert_eq!(parsed.save_data, save_data(96));
    }

    #[test]
    fn reads_a_tail_that_was_not_taken() {
        let text = mods_report(&save_data(96), None).replace('\n', "\r\n");
        assert_eq!(parse(&text).unwrap().save_tail, SaveTail::NotTaken);
    }

    #[test]
    fn refuses_a_cut_off_tail() {
        let text = mods_report(&save_data(96), Some(&tail()[..SAVE_TAIL_LEN - 16]));
        let err = parse(&text).unwrap_err();
        assert!(err.contains("the save tail has 864 bytes, expected 880"), "{err}");
    }

    #[test]
    fn mods_off_is_not_mods_on() {
        let text = mods_report(&save_data(96), Some(&tail()))
            .replace("All mods on", "All mods off (mods did not touch the game)");
        assert!(!parse(&text).unwrap().mods_on);
    }

    #[test]
    fn windows_line_endings_work() {
        let text = report(96, 4, &save_data(96)).replace('\n', "\r\n");
        assert_eq!(parse(&text).unwrap().room, 96);
    }

    #[test]
    fn refuses_title_screen_reports() {
        let err = parse(&report(0, -1, &save_data(0))).unwrap_err();
        assert!(err.contains("title screen"), "{err}");
    }

    #[test]
    fn refuses_reports_from_older_builds() {
        let text = "Game: room 96, disc 1\nStory: story day 4 (next 4)\n\n\
                    == Story progress (save file bytes from 0x13E4) ==\n13E4: 00 01\n";
        let err = parse(text).unwrap_err();
        assert!(err.contains("older build"), "{err}");
    }

    #[test]
    fn refuses_files_that_are_not_reports() {
        let err = parse("hello").unwrap_err();
        assert!(err.contains("doesn't look like a crash report"), "{err}");
    }

    #[test]
    fn refuses_missing_lines() {
        let text = report(96, 4, &save_data(96));
        let text: String = text.lines().filter(|l| !l.starts_with("0200:")).map(|l| format!("{l}\n")).collect();
        let err = parse(&text).unwrap_err();
        assert!(err.contains("jumps from 0x0200 to 0x0220"), "{err}");
    }

    #[test]
    fn refuses_cut_off_save_data() {
        let data = save_data(96);
        let err = parse(&report(96, 4, &data[..SAVE_DATA_LEN - 12])).unwrap_err();
        assert!(err.contains("expected 7052"), "{err}");
    }

    #[test]
    fn refuses_bad_bytes() {
        let text = report(96, 4, &save_data(96)).replacen("0100: 00", "0100: 0G", 1);
        let err = parse(&text).unwrap_err();
        assert!(err.contains("bad byte \"0G\""), "{err}");
    }

    #[test]
    fn reads_the_mods_record_list() {
        // As the port writes it: the records line, then one line per record, then the loaded save's line.
        let list = "Mods' save data (the room snapshot): 3 record(s), 132 of 744 bytes, writer mod API 1 / save writer 1\r\n\
                    \x20 loaded save: written by mod API 1 / save writer 1\r\n\
                    \x20 4EAF78D1 pe_qol.keys (pe_qol) v1, 68 bytes, live\r\n\
                    \x20 12345678 other_mod.state (other_mod) v3, 40 bytes, carried: mod off\r\n\
                    \x20 0BADF00D (unknown) v1, 8 bytes, carried: mod not in this build\r\n";
        let text = mods_report(&save_data(96), Some(&tail())).replace("All mods on\n", &format!("All mods on\n{list}"));
        let records = parse(&text).unwrap().mod_records;
        assert_eq!(records.len(), 2);
        assert_eq!(
            records[0],
            NamedRecord {
                id: 0x4EAF78D1,
                name: "pe_qol.keys".into(),
                owner: Some("pe_qol".into()),
                state: "live".into(),
            }
        );
        assert_eq!(records[1].name, "other_mod.state");
        assert_eq!(records[1].state, "carried: mod off");
        assert!(parse(&mods_report(&save_data(96), Some(&tail()))).unwrap().mod_records.is_empty());
    }

    #[test]
    fn refuses_room_that_does_not_match_the_save() {
        let err = parse(&report(5, 4, &save_data(96))).unwrap_err();
        assert!(err.contains("says room 96"), "{err}");
    }
}

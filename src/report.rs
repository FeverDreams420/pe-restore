//! Reads a crash-*.txt report from the PC port.

use pe_save_editor::crc::{SAVE_BLOCK_SIZE, SAVE_CRC_OFFSET};
use pe_save_editor::save::RT_OFFSET;

const SAVE_DATA_HEADER: &str = "== Save data (save file bytes from 0x0100) ==";
const SAVE_TAIL_HEADER: &str = "== Save tail (save file bytes from 0x1C90) ==";
const OLD_STORY_HEADER: &str = "== Story progress";
/// Newer builds take the save data whole when each room starts and write that
/// copy, with this section in front of it.
const SNAPSHOT_HEADER: &str = "== Room snapshot ==";
const SNAPSHOT_TAKEN: &str = "Taken when room ";

/// Number of save file bytes a report holds: 0x0100..0x1C8B (everything but
/// the card header/icon in front and the CRC at the end).
pub const SAVE_DATA_LEN: usize = SAVE_CRC_OFFSET - RT_OFFSET;

/// The save's free tail after the CRC, where mods keep their per-save data
/// (Junk, Catalog, key item pocket...). Its layout depends on the mods, so
/// it is copied as it is, never read.
pub const SAVE_TAIL_OFFSET: usize = SAVE_CRC_OFFSET + 4;
pub const SAVE_TAIL_LEN: usize = SAVE_BLOCK_SIZE - SAVE_TAIL_OFFSET;

/// The report's Save tail section (builds with mods only).
#[derive(Debug, PartialEq)]
pub enum SaveTail {
    /// File bytes 0x1C90..0x1FFF at the crash.
    Data(Vec<u8>),
    /// The section is there but empty: mod code was running when it was taken.
    NotTaken,
    /// No section: a build without mods, or one older than the section.
    Missing,
}

/// What the tool needs from a crash report.
#[derive(Debug)]
pub struct CrashReport {
    /// The save data's room: the room snapshot's, else "Game: room N".
    pub room: i16,
    /// The room snapshot's story day, else "story day S" from the Story line;
    /// -1 means no game was loaded.
    pub story_day: i32,
    /// Room snapshot: the save data is from the start of the room, this many
    /// seconds before the crash. None: an older build, data from the crash itself.
    pub seconds_before: Option<u32>,
    /// Save file bytes 0x0100..0x1C8B, in order.
    pub save_data: Vec<u8>,
    pub save_tail: SaveTail,
    /// The Mods section says "All mods on".
    pub mods_on: bool,
}

pub fn parse(text: &str) -> Result<CrashReport, String> {
    let mut room = find_room(text)?;

    let lines: Vec<&str> = text.lines().collect();
    let snapshot = match lines.iter().position(|l| l.trim_end() == SNAPSHOT_HEADER) {
        None => None,
        Some(i) => Some(read_snapshot(lines.get(i + 1).copied().unwrap_or(""))?),
    };
    let Some(start) = lines.iter().position(|l| l.trim_end() == SAVE_DATA_HEADER) else {
        if lines.iter().any(|l| l.starts_with(OLD_STORY_HEADER)) {
            return Err("this report was made by an older build: it only has the story \
                        progress, not the full save data. Make a new report with a newer build."
                .into());
        }
        return Err("this report has no \"== Save data ==\" section.".into());
    };

    let mut story_day = find_story_day(text)?;
    if let Some(s) = &snapshot {
        room = s.room;
        story_day = s.story_day;
    }
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
        let line = if snapshot.is_some() { "Room snapshot" } else { "Game line" };
        return Err(format!("the save data says room {saved_room} but the {line} says room {room}."));
    }

    let seconds_before = snapshot.map(|s| s.seconds_before);
    Ok(CrashReport { room, story_day, seconds_before, save_data, save_tail, mods_on })
}

struct Snapshot {
    room: i16,
    story_day: i32,
    seconds_before: u32,
}

/// "Taken when room 96 started, story day 4, 41 seconds before the crash. ..."
fn read_snapshot(line: &str) -> Result<Snapshot, String> {
    if line.starts_with("None") {
        return Err("the game crashed before any room had started, so there is no save data to restore.".into());
    }
    let bad = || format!("can't read the Room snapshot line \"{}\".", line.trim_end());
    let rest = line.strip_prefix(SNAPSHOT_TAKEN).ok_or_else(bad)?;
    let room = leading_int(rest).ok_or_else(bad)?;
    let rest = rest.split_once(", story day ").ok_or_else(bad)?.1;
    let story_day = leading_int(rest).ok_or_else(bad)?;
    let rest = rest.split_once(", ").ok_or_else(bad)?.1;
    let seconds_before = leading_int(rest).ok_or_else(bad)?;
    Ok(Snapshot { room, story_day, seconds_before })
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

    /// A newer build's report: the save data comes from the start of room
    /// `room`, after Aya walked on to room 5 and the day moved on.
    fn snapshot_report(room: i16, story_day: i32, data: &[u8]) -> String {
        report(5, 6, data).replace(
            &format!("\n{SAVE_DATA_HEADER}"),
            &format!(
                "{SNAPSHOT_HEADER}\nTaken when room {room} started, story day {story_day}, 41 seconds before \
                 the crash. The save data below is all from that moment.\n\n{SAVE_DATA_HEADER}"
            ),
        )
    }

    #[test]
    fn reads_a_room_snapshot() {
        let text = snapshot_report(96, 4, &save_data(96)).replace('\n', "\r\n");
        let parsed = parse(&text).unwrap();
        assert_eq!((parsed.room, parsed.story_day, parsed.seconds_before), (96, 4, Some(41)));
        assert_eq!(parsed.save_data, save_data(96));
    }

    #[test]
    fn older_reports_have_no_snapshot() {
        assert_eq!(parse(&report(96, 4, &save_data(96))).unwrap().seconds_before, None);
    }

    #[test]
    fn refuses_a_snapshot_room_that_does_not_match_the_save() {
        let err = parse(&snapshot_report(97, 4, &save_data(96))).unwrap_err();
        assert!(err.contains("the Room snapshot says room 97"), "{err}");
    }

    #[test]
    fn refuses_a_title_screen_snapshot() {
        let err = parse(&snapshot_report(0, -1, &save_data(0))).unwrap_err();
        assert!(err.contains("title screen"), "{err}");
    }

    #[test]
    fn refuses_reports_from_before_any_room() {
        let text = "== What happened ==\nGame: room -1, disc 1\nStory: story day -1 (next -1)\n\n\
                    == Room snapshot ==\nNone: no room had started yet, so there is no save data.\n";
        let err = parse(text).unwrap_err();
        assert!(err.contains("before any room had started"), "{err}");
    }

    #[test]
    fn refuses_a_bad_snapshot_line() {
        let text = snapshot_report(96, 4, &save_data(96)).replace("story day 4,", "day 4,");
        let err = parse(&text).unwrap_err();
        assert!(err.contains("can't read the Room snapshot line"), "{err}");
    }

    #[test]
    fn refuses_room_that_does_not_match_the_save() {
        let err = parse(&report(5, 4, &save_data(96))).unwrap_err();
        assert!(err.contains("says room 96"), "{err}");
    }
}

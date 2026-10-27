//! Reads a crash-*.txt report from the PC port.

use pe_save_editor::crc::SAVE_CRC_OFFSET;
use pe_save_editor::save::RT_OFFSET;

const SAVE_DATA_HEADER: &str = "== Save data (save file bytes from 0x0100) ==";
const OLD_STORY_HEADER: &str = "== Story progress";

/// Number of save file bytes a report holds: 0x0100..0x1C8B (everything but
/// the card header/icon in front and the CRC at the end).
pub const SAVE_DATA_LEN: usize = SAVE_CRC_OFFSET - RT_OFFSET;

/// What the tool needs from a crash report.
#[derive(Debug)]
pub struct CrashReport {
    /// "Game: room N".
    pub room: i16,
    /// "story day S" from the Story line; -1 means no game was loaded.
    pub story_day: i32,
    /// Save file bytes 0x0100..0x1C8B, in order.
    pub save_data: Vec<u8>,
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

    let save_data = read_save_data(&lines[start + 1..])?;

    // Save_BuildHeader puts the current room in the save, so it must match.
    let saved_room = i16::from_le_bytes([save_data[0x5E], save_data[0x5F]]);
    if saved_room != room {
        return Err(format!("the save data says room {saved_room} but the Game line says room {room}."));
    }

    Ok(CrashReport { room, story_day, save_data })
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

/// Reads "OOOO: xx xx .." lines up to the next blank line or section.
fn read_save_data(lines: &[&str]) -> Result<Vec<u8>, String> {
    let mut data = Vec::with_capacity(SAVE_DATA_LEN);
    for line in lines {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with("==") {
            break;
        }
        let (offset, bytes) = line.split_once(": ").ok_or_else(|| format!("bad line in the save data: \"{line}\"."))?;
        let offset =
            usize::from_str_radix(offset, 16).map_err(|_| format!("bad offset in the save data: \"{line}\"."))?;
        let expected = RT_OFFSET + data.len();
        if offset != expected {
            return Err(format!("the save data jumps from 0x{expected:04X} to 0x{offset:04X} (lines missing)."));
        }
        for byte in bytes.split_whitespace() {
            let value = u8::from_str_radix(byte, 16)
                .ok()
                .filter(|_| byte.len() == 2)
                .ok_or_else(|| format!("bad byte \"{byte}\" in the save data line {offset:04X}."))?;
            data.push(value);
        }
    }
    if data.len() != SAVE_DATA_LEN {
        return Err(format!(
            "the save data has {} bytes, expected {} (0x0100..0x{:04X}).",
            data.len(),
            SAVE_DATA_LEN,
            SAVE_CRC_OFFSET - 1
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
        for (i, chunk) in data.chunks(32).enumerate() {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
            text += &format!("{:04X}: {}\n", RT_OFFSET + i * 32, hex.join(" "));
        }
        text + "\n== Log of this run ==\nlog line\n"
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
    fn refuses_room_that_does_not_match_the_save() {
        let err = parse(&report(5, 4, &save_data(96))).unwrap_err();
        assert!(err.contains("says room 96"), "{err}");
    }
}

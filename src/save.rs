//! Builds the 8192-byte card save from a crash report.

use pe_save_editor::charset::format_sjis_title_bytes;
use pe_save_editor::crc::{SAVE_BLOCK_SIZE, SAVE_CRC_OFFSET, update_save_crc16};
use pe_save_editor::save::{PeSaveSlot, RT_OFFSET};
use pe_save_editor::tables::save_header_location_name;

use crate::mods::{self, Layout, ModData, NamedRecord};
use crate::report::{CrashReport, SAVE_DATA_LEN, SAVE_TAIL_OFFSET, SaveTail};

/// Card header (magic, icon, palette) comes from a sample save of the save editor.
/// Only the first 0x100 bytes are used; the title is rewritten.
const TEMPLATE: &[u8; SAVE_BLOCK_SIZE] =
    include_bytes!("../../ParasiteEveNative/fork-pr-crash/save-editor/samples/bu00/BASLUS-006620000000A");

const HEADER_LEN: usize = RT_OFFSET;
const TITLE: std::ops::Range<usize> = 0x04..0x44;

const FULL_RECOVER: u8 = 0x0A;
const HEALING_ITEMS: usize = 6;

/// Card 1 slot `slot_index` (0-based) file name of a normal (not Clear Data) save.
pub fn file_name(slot_index: usize) -> String {
    format!("BASLUS-006620000000{}", (b'A' + slot_index as u8) as char)
}

/// Where the built save's mod data (the tail after the CRC) comes from.
#[derive(Debug, PartialEq)]
pub enum TailFrom {
    /// The report's Save tail section: the mods' data at the start of the room
    /// (newer builds) or at the crash.
    Report,
    /// The save that was in the slot: the mods' data at that save.
    OldSave,
    /// Neither has any: the tail stays empty.
    Nothing,
}

/// The tail of a save file already in the slot, when it is a good save that
/// holds mod data.
pub fn old_tail(bytes: &[u8]) -> Option<&[u8]> {
    let good = bytes.len() == SAVE_BLOCK_SIZE
        && PeSaveSlot::from_bytes(bytes, 0, String::new()).is_ok_and(|s| s.is_crc_valid());
    let tail = bytes.get(SAVE_TAIL_OFFSET..)?;
    (good && tail.iter().any(|&b| b != 0)).then_some(tail)
}

pub fn tail_from(report: &CrashReport, old_tail: Option<&[u8]>) -> TailFrom {
    match (&report.save_tail, old_tail) {
        (SaveTail::Data(_), _) => TailFrom::Report,
        (_, Some(_)) => TailFrom::OldSave,
        _ => TailFrom::Nothing,
    }
}

/// What the player should know about the mods' data in the built save, if anything.
pub fn tail_warning(report: &CrashReport, from: &TailFrom) -> Option<String> {
    const MOD_DATA: &str = "the mods' data (Junk, Mod Catalog, Requisition, key item pocket, bag sort)";
    let why = if report.save_tail == SaveTail::NotTaken {
        "The report has no mod data: mod code was running when it was taken."
    } else {
        "The report has no mod data."
    };
    match from {
        TailFrom::Report => None,
        TailFrom::OldSave => Some(format!(
            "{why} So {MOD_DATA} is kept from the save that is in the slot now: it is from that save, \
             not from the crash. Key items moved into or out of the pocket since then may be missing or doubled."
        )),
        TailFrom::Nothing if report.mods_on || report.save_tail == SaveTail::NotTaken => Some(format!(
            "{why} There is no save with mod data in the slot to keep it from, so {MOD_DATA} will be missing. \
             Key items that were in the pocket will be gone, which can block story doors."
        )),
        TailFrom::Nothing => None,
    }
}

/// The mods' data the built save will hold, read to show it (the bytes are copied as they are),
/// and where it comes from. None when it holds none.
pub fn mods_in_save(report: &CrashReport, old_tail: Option<&[u8]>) -> Option<(TailFrom, ModData)> {
    let from = tail_from(report, old_tail);
    match (&report.save_tail, &from) {
        (SaveTail::Data(tail), _) => Some((from, mods::read(tail, &report.mod_records))),
        (_, TailFrom::OldSave) => {
            // The report's names fit any save's records; its states describe the report's data, not that save.
            let names: Vec<NamedRecord> =
                report.mod_records.iter().map(|n| NamedRecord { state: String::new(), ..n.clone() }).collect();
            let mut data = mods::read(old_tail?, &names);
            data.records.iter_mut().for_each(|r| r.state = None);
            Some((from, data))
        }
        _ => None,
    }
}

/// What the player should know about what the mods' data holds: damaged records, a newer
/// format. Each such mod starts from its defaults in the game.
pub fn mods_warning(data: &ModData) -> Option<String> {
    let label = |r: &mods::Record| r.name.clone().unwrap_or_else(|| format!("record {:08X}", r.id));
    match &data.layout {
        Layout::Newer(format) => Some(format!(
            "The mods' data is in a newer format ({format}) than this tool knows. It is kept as it is; \
             a build that can't read it starts the mods from their defaults and keeps it for a newer build."
        )),
        Layout::Unreadable => {
            Some("The mods' data can't be read. The game starts every mod from its defaults with this save.".into())
        }
        _ => {
            let damaged: Vec<String> = data.records.iter().filter(|r| !r.crc_ok).map(label).collect();
            (!damaged.is_empty()).then(|| {
                format!(
                    "Part of the mods' data was written over: {}. Those start from their defaults in the game; \
                     the rest loads.",
                    damaged.join(", ")
                )
            })
        }
    }
}

/// The save the game would have written in `slot_index` at the start of the crash room
/// (newer builds) or at the moment of the crash (older ones).
/// `old_tail` (see `old_tail`) is the mods' data of the save already in that
/// slot, used when the report has none.
pub fn build(report: &CrashReport, slot_index: usize, old_tail: Option<&[u8]>) -> Result<PeSaveSlot, String> {
    let mut block = vec![0u8; SAVE_BLOCK_SIZE];
    block[..HEADER_LEN].copy_from_slice(&TEMPLATE[..HEADER_LEN]);
    block[RT_OFFSET..RT_OFFSET + SAVE_DATA_LEN].copy_from_slice(&report.save_data);
    // Copied as it is: the layout belongs to the mods. The CRC below covers it.
    let tail = match &report.save_tail {
        SaveTail::Data(tail) => Some(tail.as_slice()),
        _ => old_tail,
    };
    if let Some(tail) = tail {
        block[SAVE_TAIL_OFFSET..].copy_from_slice(tail);
    }

    let rt = &block[RT_OFFSET..];
    let play_time = i32::from_le_bytes(rt[0x08..0x0C].try_into().unwrap());
    let card_day = i16::from_le_bytes([rt[0x5C], rt[0x5D]]);
    let location = save_header_location_name(report.room, false);
    block[TITLE].copy_from_slice(&format_sjis_title_bytes(slot_index + 1, play_time, card_day, Some(location), false));

    update_save_crc16(&mut block);
    let slot = PeSaveSlot::from_bytes(&block, slot_index, file_name(slot_index))
        .map_err(|e| format!("the save editor can't read the built save: {e}"))?;
    if !slot.is_crc_valid() {
        return Err("the built save has a bad CRC.".into());
    }
    Ok(slot)
}

/// Makes Aya strong for testing: level 99, 999 HP, a strong gun and armour in the
/// bag (not equipped) and at least 6 Full Recovers. Items only go into empty bag
/// slots, so nothing she carries (key items) is removed.
pub fn apply_test_setup(slot: &mut PeSaveSlot) -> Result<(), String> {
    slot.apply_cheat_max_aya();
    slot.apply_cheat_give_ultimate_loadout().map_err(|e| e.to_string())?;
    let mut have = slot.inventory_items.iter().filter(|&&v| v == FULL_RECOVER as i16).count();
    while have < HEALING_ITEMS {
        if slot.add_item_to_inventory(FULL_RECOVER).is_err() {
            break;
        }
        have += 1;
    }
    Ok(())
}

/// The 8192 bytes to write to the card, with a fresh CRC.
pub fn to_bytes(slot: &mut PeSaveSlot) -> Vec<u8> {
    let bytes = slot.to_bytes();
    debug_assert_eq!(bytes.len(), SAVE_BLOCK_SIZE);
    debug_assert!(bytes[SAVE_CRC_OFFSET + 2..SAVE_CRC_OFFSET + 4] == [0, 0]);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use pe_save_editor::charset::decode_sjis_title;
    use pe_save_editor::crc::calc_save_crc16;

    /// Report data taken from the template's own save, moved to room 96.
    fn report() -> CrashReport {
        let mut save_data = TEMPLATE[RT_OFFSET..SAVE_CRC_OFFSET].to_vec();
        save_data[0x5E..0x60].copy_from_slice(&96i16.to_le_bytes());
        CrashReport {
            room: 96,
            story_day: 4,
            seconds_before: None,
            save_data,
            save_tail: SaveTail::Missing,
            mods_on: false,
            mod_records: Vec::new(),
        }
    }

    #[test]
    fn keeps_report_bytes_and_template_header() {
        let r = report();
        let mut slot = build(&r, 0, None).unwrap();
        let bytes = to_bytes(&mut slot);
        assert_eq!(&bytes[RT_OFFSET..SAVE_CRC_OFFSET], &r.save_data[..]);
        assert_eq!(&bytes[..4], &TEMPLATE[..4]);
        assert_eq!(&bytes[0x44..HEADER_LEN], &TEMPLATE[0x44..HEADER_LEN]);
        assert!(bytes[SAVE_CRC_OFFSET + 4..].iter().all(|&b| b == 0));
        assert_eq!(
            calc_save_crc16(&bytes) as u32,
            u32::from_le_bytes(bytes[SAVE_CRC_OFFSET..SAVE_CRC_OFFSET + 4].try_into().unwrap())
        );
    }

    #[test]
    fn title_names_slot_and_room() {
        let mut slot = build(&report(), 2, None).unwrap();
        let bytes = to_bytes(&mut slot);
        let title = decode_sjis_title(&bytes[TITLE]).1;
        assert!(title.starts_with("PE-03/"), "{title}");
        assert!(title.ends_with("Soho"), "{title}");
        assert_eq!(slot.filename, "BASLUS-006620000000C");
    }

    #[test]
    fn test_setup_keeps_items_and_adds_healing() {
        let mut slot = build(&report(), 0, None).unwrap();
        let before: Vec<i16> = slot.inventory_items.iter().copied().filter(|&v| v != 0).collect();
        apply_test_setup(&mut slot).unwrap();
        let bytes = to_bytes(&mut slot);
        let again = PeSaveSlot::from_bytes(&bytes, 0, file_name(0)).unwrap();
        assert!(again.is_crc_valid());
        assert_eq!(again.display_level(), 99);
        assert_eq!(again.current_hp, 999);
        for item in before {
            assert!(again.inventory_items.contains(&item), "lost item {item:#X}");
        }
        let healing = again.inventory_items.iter().filter(|&&v| v == FULL_RECOVER as i16).count();
        assert!(healing >= HEALING_ITEMS, "{healing}");
    }

    fn tail(seed: u8) -> Vec<u8> {
        (0..crate::report::SAVE_TAIL_LEN).map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed)).collect()
    }

    /// The bytes of a good save in the slot whose tail is `tail`.
    fn old_save(tail: &[u8]) -> Vec<u8> {
        let mut bytes = TEMPLATE.to_vec();
        bytes[SAVE_TAIL_OFFSET..].copy_from_slice(tail);
        update_save_crc16(&mut bytes);
        bytes
    }

    fn written_tail(r: &CrashReport, old: Option<&[u8]>, strong: bool) -> Vec<u8> {
        let mut slot = build(r, 0, old).unwrap();
        if strong {
            apply_test_setup(&mut slot).unwrap();
        }
        let bytes = to_bytes(&mut slot);
        let again = PeSaveSlot::from_bytes(&bytes, 0, file_name(0)).unwrap();
        assert!(again.is_crc_valid());
        bytes[SAVE_TAIL_OFFSET..].to_vec()
    }

    #[test]
    fn copies_the_report_tail_over_the_old_one() {
        let r = CrashReport { save_tail: SaveTail::Data(tail(1)), mods_on: true, ..report() };
        let old = old_save(&tail(2));
        assert_eq!(written_tail(&r, old_tail(&old), false), tail(1));
        assert_eq!(written_tail(&r, old_tail(&old), true), tail(1));
        assert_eq!(tail_from(&r, old_tail(&old)), TailFrom::Report);
        assert_eq!(tail_warning(&r, &TailFrom::Report), None);
    }

    #[test]
    fn keeps_the_old_tail_when_the_report_has_none() {
        for save_tail in [SaveTail::NotTaken, SaveTail::Missing] {
            let not_taken = save_tail == SaveTail::NotTaken;
            let r = CrashReport { save_tail, mods_on: true, ..report() };
            let old = old_save(&tail(2));
            assert_eq!(written_tail(&r, old_tail(&old), false), tail(2));
            let from = tail_from(&r, old_tail(&old));
            assert_eq!(from, TailFrom::OldSave);
            let warning = tail_warning(&r, &from).unwrap();
            assert!(warning.contains("kept from the save that is in the slot now"), "{warning}");
            assert_eq!(warning.contains("mod code was running"), not_taken, "{warning}");
        }
    }

    #[test]
    fn warns_when_mod_data_is_lost() {
        let r = CrashReport { mods_on: true, ..report() };
        assert!(written_tail(&r, None, false).iter().all(|&b| b == 0));
        let warning = tail_warning(&r, &tail_from(&r, None)).unwrap();
        assert!(warning.contains("will be missing") && warning.contains("story doors"), "{warning}");
        // A build without mods: nothing to say.
        assert_eq!(tail_warning(&report(), &tail_from(&report(), None)), None);
    }

    #[test]
    fn shows_the_mods_data_the_save_will_hold() {
        let loot = {
            let mut d = vec![0u8; 36];
            d[..4].copy_from_slice(&122u32.to_le_bytes());
            d
        };
        let crash = mods::tests::container(&[("pe_economy.loot", 1, &loot)], &[0; 136]);
        let names = vec![NamedRecord {
            id: mods::fnv1a("pe_economy.loot"),
            name: "pe_economy.loot".into(),
            owner: Some("pe_economy".into()),
            state: "live".into(),
        }];
        let r = CrashReport { save_tail: SaveTail::Data(crash.clone()), mods_on: true, mod_records: names, ..report() };
        let (from, data) = mods_in_save(&r, None).unwrap();
        assert_eq!(from, TailFrom::Report);
        assert_eq!(mods::summary(&data.records[0]).unwrap(), "122 Junk");
        assert_eq!(data.records[0].state.as_deref(), Some("live"));
        assert_eq!(mods_warning(&data), None);
        // Built as it is: the bytes go in unchanged.
        assert_eq!(written_tail(&r, None, false), crash);

        // No mods' data in the report: the slot's save's, named, without the crash's states.
        let r2 = CrashReport { save_tail: SaveTail::NotTaken, ..r };
        let old = old_save(&crash);
        let (from, data) = mods_in_save(&r2, old_tail(&old)).unwrap();
        assert_eq!(from, TailFrom::OldSave);
        assert_eq!(data.records[0].name.as_deref(), Some("pe_economy.loot"));
        assert_eq!(data.records[0].state, None);
        assert!(mods_in_save(&report(), None).is_none());
    }

    /// With PE_CRASH_REPORT=<a crash-*.txt from a mods build with the host's records>: the
    /// report's records are read, named and summed up, and the built save keeps them byte for byte.
    #[test]
    fn real_crash_report() {
        let Ok(path) = std::env::var("PE_CRASH_REPORT") else { return };
        let r = crate::report::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let (from, data) = mods_in_save(&r, None).expect("the report has mods' data");
        assert_eq!(from, TailFrom::Report);
        assert!(matches!(data.layout, Layout::Records { container_ok: true, .. }), "{:?}", data.layout);
        assert_eq!(mods_warning(&data), None);
        for rec in &data.records {
            assert!(rec.crc_ok && rec.name.is_some() && rec.owner.is_some() && rec.state.is_some(), "{rec:?}");
            println!(
                "{} {} {:?} {:?}",
                rec.owner.as_deref().unwrap(),
                rec.name.as_deref().unwrap(),
                mods::summary(rec),
                rec.state
            );
        }
        let SaveTail::Data(tail) = &r.save_tail else { unreachable!() };
        assert_eq!(&written_tail(&r, None, false), tail);
    }

    #[test]
    fn warns_about_damaged_or_newer_mods_data() {
        let mut tail = mods::tests::container(&[("pe_qol.keys", 1, &[0u8; 68])], &[0; 136]);
        tail[20 + 12] ^= 1;
        let warning = mods_warning(&mods::read(&tail, &[])).unwrap();
        assert!(warning.contains("pe_qol.keys") && warning.contains("defaults"), "{warning}");
        tail[20 + 12] ^= 1;
        tail[4] = 2;
        assert!(mods_warning(&mods::read(&tail, &[])).unwrap().contains("newer format"));
    }

    #[test]
    fn old_tail_only_from_good_saves_with_mod_data() {
        assert_eq!(old_tail(&old_save(&tail(2))), Some(tail(2).as_slice()));
        assert_eq!(old_tail(&old_save(&vec![0; crate::report::SAVE_TAIL_LEN])), None);
        let mut damaged = old_save(&tail(2));
        damaged[0x200] ^= 1;
        assert_eq!(old_tail(&damaged), None);
        assert_eq!(old_tail(&damaged[..100]), None);
    }
}

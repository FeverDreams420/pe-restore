//! Builds the 8192-byte card save from a crash report.

use pe_save_editor::charset::format_sjis_title_bytes;
use pe_save_editor::crc::{SAVE_BLOCK_SIZE, SAVE_CRC_OFFSET, update_save_crc16};
use pe_save_editor::save::{PeSaveSlot, RT_OFFSET};
use pe_save_editor::tables::save_header_location_name;

use crate::report::{CrashReport, SAVE_DATA_LEN};

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

/// The save the game would have written in `slot_index` at the moment of the crash.
pub fn build(report: &CrashReport, slot_index: usize) -> Result<PeSaveSlot, String> {
    let mut block = vec![0u8; SAVE_BLOCK_SIZE];
    block[..HEADER_LEN].copy_from_slice(&TEMPLATE[..HEADER_LEN]);
    block[RT_OFFSET..RT_OFFSET + SAVE_DATA_LEN].copy_from_slice(&report.save_data);

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
        CrashReport { room: 96, story_day: 4, save_data }
    }

    #[test]
    fn keeps_report_bytes_and_template_header() {
        let r = report();
        let mut slot = build(&r, 0).unwrap();
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
        let mut slot = build(&report(), 2).unwrap();
        let bytes = to_bytes(&mut slot);
        let title = decode_sjis_title(&bytes[TITLE]).1;
        assert!(title.starts_with("PE-03/"), "{title}");
        assert!(title.ends_with("Soho"), "{title}");
        assert_eq!(slot.filename, "BASLUS-006620000000C");
    }

    #[test]
    fn test_setup_keeps_items_and_adds_healing() {
        let mut slot = build(&report(), 0).unwrap();
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
}

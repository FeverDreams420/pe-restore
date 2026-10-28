//! Reads the mods' data in a save's free tail (file bytes 0x1C90..0x1FFF), to show what a
//! restored save will hold. The bytes themselves are always copied as they are.
//!
//! The port's mod host keeps every mod's per-save data there as named records (its
//! `mods/host/pe_save.h`): a "PEMC" container whose bytes run around a window at tail offsets
//! 0x200..0x287, where the old key pocket and bag sort blocks stay for builds without records.
//! Saves from before the records hold each mod's own fixed block instead (PELT, PECT, PESH,
//! PKEY, SORT).

use crate::report::SAVE_TAIL_LEN;

const WINDOW: std::ops::Range<usize> = 0x200..0x288;
const ROOM: usize = SAVE_TAIL_LEN - (WINDOW.end - WINDOW.start);
const HEADER_LEN: usize = 20;
const RECORD_HEADER_LEN: usize = 12;
const FORMAT: u8 = 1;

/// The old fixed blocks: the record that replaced each, its tail offset, size and magic.
const OLD_BLOCKS: [(&str, usize, usize, &[u8; 4]); 5] = [
    ("pe_economy.loot", 0x000, 0x020, b"PELT"),
    ("pe_economy.catalog", 0x040, 0x148, b"PECT"),
    ("pe_economy.shop", 0x1C0, 0x014, b"PESH"),
    ("pe_qol.keys", 0x200, 0x054, b"PKEY"),
    ("pe_qol.sort", 0x260, 0x028, b"SORT"),
];

/// One mod's piece of per-save data.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// FNV-1a of the record's name.
    pub id: u32,
    /// "pe_qol.keys"; None when neither the report nor the known records name it.
    pub name: Option<String>,
    /// The mod it belongs to ("pe_qol"), when the report says.
    pub owner: Option<String>,
    /// The record's layout version; 0 for an old fixed block.
    pub version: u16,
    pub data: Vec<u8>,
    /// Its own CRC matches (old fixed blocks: always true).
    pub crc_ok: bool,
    /// What the report says about it at the crash ("live", "carried: mod off" ...).
    pub state: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Layout {
    /// No mods' data at all.
    Empty,
    /// The host's records; `container_ok` is false when part of it was written over.
    Records { container_ok: bool, api: u16, writer: u16 },
    /// A save from before the records: each mod's own block.
    OldBlocks,
    /// A container format newer than this tool reads (the game keeps it as it is).
    Newer(u8),
    /// A container whose header can't be read, or bytes nobody's layout explains.
    Unreadable,
}

#[derive(Debug, PartialEq)]
pub struct ModData {
    pub layout: Layout,
    pub records: Vec<Record>,
}

/// A record the crash report names (its Mods section lists the snapshot's records).
#[derive(Debug, Clone, PartialEq)]
pub struct NamedRecord {
    pub id: u32,
    pub name: String,
    pub owner: Option<String>,
    pub state: String,
}

pub fn fnv1a(name: &str) -> u32 {
    name.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

fn crc32_add(mut crc: u32, bytes: &[u8]) -> u32 {
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    crc
}

fn crc32(bytes: &[u8]) -> u32 {
    !crc32_add(!0, bytes)
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// The container's bytes, in order: the tail without the window.
fn gather(tail: &[u8]) -> Vec<u8> {
    let mut lin = tail[..WINDOW.start].to_vec();
    lin.extend_from_slice(&tail[WINDOW.end..]);
    lin
}

/// The mods' data in `tail` (SAVE_TAIL_LEN bytes), named from `names` (the report's list)
/// where it has them, else from the records this tool knows.
pub fn read(tail: &[u8], names: &[NamedRecord]) -> ModData {
    let mut data = read_unnamed(tail);
    for r in &mut data.records {
        if let Some(n) = names.iter().find(|n| n.id == r.id) {
            r.name = Some(n.name.clone());
            r.owner = n.owner.clone();
            r.state = Some(n.state.clone());
        } else if r.name.is_none() {
            r.name = OLD_BLOCKS.iter().find(|b| fnv1a(b.0) == r.id).map(|b| b.0.to_string());
        }
        if r.owner.is_none() {
            r.owner = r.name.as_deref().and_then(|n| n.split_once('.')).map(|(m, _)| m.to_string());
        }
    }
    data
}

fn read_unnamed(tail: &[u8]) -> ModData {
    if tail.len() != SAVE_TAIL_LEN || tail.iter().all(|&b| b == 0) {
        return ModData { layout: Layout::Empty, records: Vec::new() };
    }
    if &tail[..4] != b"PEMC" {
        let records: Vec<Record> = OLD_BLOCKS
            .iter()
            .filter(|(_, at, _, magic)| &tail[*at..*at + 4] == *magic)
            .map(|&(name, at, size, _)| Record {
                id: fnv1a(name),
                name: Some(name.to_string()),
                owner: None,
                version: 0,
                data: tail[at..at + size].to_vec(),
                crc_ok: true,
                state: None,
            })
            .collect();
        let layout = if records.is_empty() { Layout::Unreadable } else { Layout::OldBlocks };
        return ModData { layout, records };
    }
    let (format, flags, total) = (tail[4], tail[5], u16_at(tail, 6) as usize);
    if format > FORMAT || (format == FORMAT && flags != 0) {
        return ModData { layout: Layout::Newer(format), records: Vec::new() };
    }
    if format != FORMAT || !(HEADER_LEN..=ROOM).contains(&total) {
        return ModData { layout: Layout::Unreadable, records: Vec::new() };
    }
    let mut lin = gather(tail);
    let stored = u32_at(&lin, 8);
    lin[8..12].fill(0);
    let container_ok = crc32(&lin[..total]) == stored;
    let (api, writer) = (u16_at(&lin, 16), u16_at(&lin, 18));

    let mut records = Vec::new();
    let mut at = HEADER_LEN;
    while at + RECORD_HEADER_LEN <= total {
        let size = u16_at(&lin, at + 6) as usize;
        if size > total - at - RECORD_HEADER_LEN {
            break;
        }
        let body = &lin[at + RECORD_HEADER_LEN..at + RECORD_HEADER_LEN + size];
        let crc_ok = !crc32_add(crc32_add(!0, &lin[at..at + 8]), body) == u32_at(&lin, at + 8);
        records.push(Record {
            id: u32_at(&lin, at),
            name: None,
            owner: None,
            version: u16_at(&lin, at + 4),
            data: body.to_vec(),
            crc_ok,
            state: None,
        });
        at += RECORD_HEADER_LEN + size;
    }
    // The window changed since: a build without records saved here, and its blocks win.
    if crc32(&tail[WINDOW]) != u32_at(&lin, 12) {
        for &(name, off, size, magic) in OLD_BLOCKS.iter().filter(|b| WINDOW.contains(&b.1)) {
            if &tail[off..off + 4] == magic {
                records.retain(|r| r.id != fnv1a(name));
                records.push(Record {
                    id: fnv1a(name),
                    name: Some(name.to_string()),
                    owner: None,
                    version: 0,
                    data: tail[off..off + size].to_vec(),
                    crc_ok: true,
                    state: None,
                });
            }
        }
    }
    ModData { layout: Layout::Records { container_ok, api, writer }, records }
}

/// What a PE Enhancements record holds, in a few words; None for other mods' records.
pub fn summary(r: &Record) -> Option<String> {
    let body = if r.version == 0 { r.data.get(16..)? } else { &r.data[..] };
    let name = r.name.as_deref()?;
    Some(match name {
        "pe_economy.loot" => {
            let junk = if r.version == 0 { u32_at(&r.data, 8) } else { u32_at(&r.data, 0) };
            format!("{junk} Junk")
        }
        "pe_economy.catalog" => {
            let (gun, armor) = (u32_at(body, 0).count_ones(), u32_at(body, 4).count_ones());
            format!("{gun} gun and {armor} armor effects unlocked, {} to rebuild", body.get(8)?)
        }
        "pe_economy.shop" => format!("{} Tools bought", u16_at(body, 0)),
        "pe_qol.keys" => match pocketed_keys(r).len() {
            0 => "no key items pocketed".into(),
            1 => "1 key item pocketed".into(),
            n => format!("{n} key items pocketed"),
        },
        "pe_qol.sort" => if u32_at(body, 0) == 1 { "bag sort: Auto" } else { "bag sort: Manual" }.into(),
        _ => return None,
    })
}

/// The pocketed key items' ids in a "pe_qol.keys" record.
pub fn pocketed_keys(r: &Record) -> Vec<u16> {
    let body = if r.version == 0 { r.data.get(16..).unwrap_or(&[]) } else { &r.data[..] };
    if body.len() < 4 {
        return Vec::new();
    }
    let count = (u16_at(body, 0) as usize).min(32).min((body.len() - 4) / 2);
    (0..count).map(|i| u16_at(body, 4 + i * 2)).collect()
}

/// Friendly names of the PE Enhancements mods.
pub fn mod_label(owner: &str) -> String {
    match owner {
        "pe_qol" => "Quality of life".into(),
        "pe_economy" => "Economy".into(),
        "pe_rebalance" => "Rebalance".into(),
        "pe_collection" => "Collection".into(),
        "pe_tools" => "Tools".into(),
        other => other.into(),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A tail as the port's host writes it: records sorted by id, the window as given.
    pub fn container(records: &[(&str, u16, &[u8])], window: &[u8]) -> Vec<u8> {
        let mut items: Vec<(u32, u16, &[u8])> = records.iter().map(|&(n, v, d)| (fnv1a(n), v, d)).collect();
        items.sort_by_key(|i| i.0);
        let mut body = Vec::new();
        for (id, version, data) in items {
            let mut head = Vec::new();
            head.extend_from_slice(&id.to_le_bytes());
            head.extend_from_slice(&version.to_le_bytes());
            head.extend_from_slice(&(data.len() as u16).to_le_bytes());
            let crc = !crc32_add(crc32_add(!0, &head), data);
            body.extend_from_slice(&head);
            body.extend_from_slice(&crc.to_le_bytes());
            body.extend_from_slice(data);
        }
        let total = HEADER_LEN + body.len();
        let mut lin = b"PEMC".to_vec();
        lin.extend_from_slice(&[FORMAT, 0]);
        lin.extend_from_slice(&(total as u16).to_le_bytes());
        lin.extend_from_slice(&[0; 4]);
        lin.extend_from_slice(&crc32(window).to_le_bytes());
        lin.extend_from_slice(&1u16.to_le_bytes());
        lin.extend_from_slice(&1u16.to_le_bytes());
        lin.extend_from_slice(&body);
        let crc = crc32(&lin);
        lin[8..12].copy_from_slice(&crc.to_le_bytes());
        lin.resize(ROOM, 0);
        let mut tail = lin[..WINDOW.start].to_vec();
        tail.extend_from_slice(window);
        tail.extend_from_slice(&lin[WINDOW.start..]);
        tail
    }

    fn keys(ids: &[u16]) -> Vec<u8> {
        let mut d = vec![0u8; 68];
        d[0..2].copy_from_slice(&(ids.len() as u16).to_le_bytes());
        for (i, id) in ids.iter().enumerate() {
            d[4 + i * 2..6 + i * 2].copy_from_slice(&id.to_le_bytes());
        }
        d
    }

    fn loot(junk: u32) -> Vec<u8> {
        let mut d = vec![0u8; 36];
        d[0..4].copy_from_slice(&junk.to_le_bytes());
        d
    }

    #[test]
    fn reads_records_and_names_them() {
        let tail =
            container(&[("pe_economy.loot", 1, &loot(122)), ("pe_qol.keys", 1, &keys(&[0xC8, 0xC9]))], &[0; 136]);
        let data = read(&tail, &[]);
        assert_eq!(data.layout, Layout::Records { container_ok: true, api: 1, writer: 1 });
        assert_eq!(data.records.len(), 2);
        let loot = data.records.iter().find(|r| r.name.as_deref() == Some("pe_economy.loot")).unwrap();
        assert_eq!(summary(loot).unwrap(), "122 Junk");
        assert_eq!(loot.owner.as_deref(), Some("pe_economy"));
        let keys = data.records.iter().find(|r| r.name.as_deref() == Some("pe_qol.keys")).unwrap();
        assert_eq!(pocketed_keys(keys), vec![0xC8, 0xC9]);
        assert_eq!(summary(keys).unwrap(), "2 key items pocketed");
    }

    #[test]
    fn names_any_mods_records_from_the_report() {
        let tail = container(&[("other_mod.state", 3, b"hello")], &[0; 136]);
        assert_eq!(read(&tail, &[]).records[0].name, None);
        let named = NamedRecord {
            id: fnv1a("other_mod.state"),
            name: "other_mod.state".into(),
            owner: Some("other_mod".into()),
            state: "carried: mod off".into(),
        };
        let r = &read(&tail, &[named]).records[0];
        assert_eq!(r.name.as_deref(), Some("other_mod.state"));
        assert_eq!(r.owner.as_deref(), Some("other_mod"));
        assert_eq!(r.state.as_deref(), Some("carried: mod off"));
        assert_eq!(summary(r), None);
    }

    #[test]
    fn a_damaged_record_shows_as_damaged() {
        let mut tail = container(&[("pe_economy.loot", 1, &loot(5)), ("pe_qol.keys", 1, &keys(&[0xC8]))], &[0; 136]);
        // The records are sorted by id: pe_qol.keys (0x4EAF78D1) first, its data after two headers.
        tail[HEADER_LEN + RECORD_HEADER_LEN] ^= 0xFF;
        let data = read(&tail, &[]);
        assert_eq!(data.layout, Layout::Records { container_ok: false, api: 1, writer: 1 });
        let bad: Vec<_> = data.records.iter().filter(|r| !r.crc_ok).map(|r| r.name.clone().unwrap()).collect();
        assert_eq!(bad, vec!["pe_qol.keys"]);
    }

    #[test]
    fn an_old_builds_key_pocket_in_the_window_wins() {
        let tail = container(&[("pe_qol.keys", 1, &keys(&[0xC8]))], &[0; 136]);
        let mut tail2 = tail.clone();
        let block = {
            let mut b = b"PKEY".to_vec();
            b.extend_from_slice(&[1, 0, 0, 0, 68, 0, 0, 0, 0, 0, 0, 0]);
            b.extend_from_slice(&keys(&[0xC8, 0xCB]));
            b
        };
        tail2[0x200..0x200 + block.len()].copy_from_slice(&block);
        let r = read(&tail2, &[]).records.into_iter().find(|r| r.name.as_deref() == Some("pe_qol.keys")).unwrap();
        assert_eq!(r.version, 0);
        assert_eq!(pocketed_keys(&r), vec![0xC8, 0xCB]);
        assert_eq!(pocketed_keys(&read(&tail, &[]).records[0]), vec![0xC8]);
    }

    #[test]
    fn old_fixed_blocks_newer_formats_and_empty_tails() {
        let mut tail = vec![0u8; SAVE_TAIL_LEN];
        assert_eq!(read(&tail, &[]).layout, Layout::Empty);
        tail[..4].copy_from_slice(b"PELT");
        tail[4] = 4;
        tail[8..12].copy_from_slice(&42u32.to_le_bytes());
        let data = read(&tail, &[]);
        assert_eq!(data.layout, Layout::OldBlocks);
        assert_eq!(summary(&data.records[0]).unwrap(), "42 Junk");
        let mut newer = container(&[], &[0; 136]);
        newer[4] = 2;
        assert_eq!(read(&newer, &[]).layout, Layout::Newer(2));
    }
}

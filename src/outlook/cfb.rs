use super::MsgCompoundFile;
use crate::core::Diagnostic;
use std::collections::BTreeSet;

const PARSER: &str = "grist.outlook.msg";
const MAGIC: &[u8; 8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";
const FREE_SECTOR: u32 = 0xffff_ffff;
const END_OF_CHAIN: u32 = 0xffff_fffe;
const DIFAT_SECTOR: u32 = 0xffff_fffc;
const NO_STREAM: u32 = 0xffff_ffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CfbEntryKind {
    Storage,
    Stream,
    Root,
}

#[derive(Debug, Clone)]
pub(super) struct CfbEntry {
    pub id: usize,
    pub name: String,
    pub path: String,
    pub parent_path: String,
    pub kind: CfbEntryKind,
    pub clsid: Option<String>,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct CfbFile {
    pub metadata: MsgCompoundFile,
    pub entries: Vec<CfbEntry>,
    pub diagnostics: Vec<Diagnostic>,
}

impl CfbFile {
    pub fn direct_children<'a>(
        &'a self,
        storage_path: &str,
    ) -> impl Iterator<Item = &'a CfbEntry> + 'a {
        let storage_path = storage_path.to_string();
        self.entries.iter().filter(move |entry| {
            entry.parent_path == storage_path && entry.kind != CfbEntryKind::Root
        })
    }

    pub fn entry(&self, path: &str) -> Option<&CfbEntry> {
        self.entries.iter().find(|entry| entry.path == path)
    }

    pub fn child(&self, storage_path: &str, name: &str) -> Option<&CfbEntry> {
        self.direct_children(storage_path)
            .find(move |entry| entry.name.eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Clone)]
struct DirectoryRecord {
    name: String,
    kind: Option<CfbEntryKind>,
    left: u32,
    right: u32,
    child: u32,
    clsid: Option<String>,
    start_sector: u32,
    stream_size: u64,
}

pub(super) fn parse(bytes: &[u8], max_chain_sectors: usize) -> Result<CfbFile, Box<Diagnostic>> {
    if max_chain_sectors == 0 {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "max_chain_sectors must be greater than zero",
        )));
    }
    if bytes.len() < 512 || bytes.get(..8) != Some(MAGIC) {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "input is not a Compound Binary File",
        )));
    }
    let minor_version = u16_at(bytes, 24)?;
    let major_version = u16_at(bytes, 26)?;
    if u16_at(bytes, 28)? != 0xfffe {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "Compound Binary File byte order is not little-endian",
        )));
    }
    let sector_shift = u16_at(bytes, 30)?;
    let mini_sector_shift = u16_at(bytes, 32)?;
    let sector_size = 1usize
        .checked_shl(u32::from(sector_shift))
        .ok_or_else(|| Box::new(Diagnostic::malformed(PARSER, "invalid CFB sector shift")))?;
    let mini_sector_size = 1usize
        .checked_shl(u32::from(mini_sector_shift))
        .ok_or_else(|| {
            Box::new(Diagnostic::malformed(
                PARSER,
                "invalid CFB mini-sector shift",
            ))
        })?;
    if !matches!((major_version, sector_size), (3, 512) | (4, 4096)) || mini_sector_size != 64 {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "unsupported or internally inconsistent CFB sector geometry",
        )));
    }
    if bytes.len() < sector_size {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "CFB header sector is truncated",
        )));
    }

    let num_fat_sectors = u32_at(bytes, 44)? as usize;
    let first_directory_sector = u32_at(bytes, 48)?;
    let mini_stream_cutoff = u32_at(bytes, 56)?;
    let first_minifat_sector = u32_at(bytes, 60)?;
    let num_minifat_sectors = u32_at(bytes, 64)? as usize;
    let first_difat_sector = u32_at(bytes, 68)?;
    let num_difat_sectors = u32_at(bytes, 72)? as usize;
    let mut diagnostics = Vec::new();
    let mut difat = Vec::new();
    for index in 0..109 {
        let sector = u32_at(bytes, 76 + index * 4)?;
        if sector < DIFAT_SECTOR {
            difat.push(sector);
        }
    }
    read_difat_chain(
        bytes,
        sector_size,
        first_difat_sector,
        num_difat_sectors,
        max_chain_sectors,
        &mut difat,
        &mut diagnostics,
    );
    if difat.len() < num_fat_sectors {
        diagnostics.push(partial(
            "outlook.msg.cfb.fat_truncated",
            "CFB declares more FAT sectors than the bounded DIFAT exposes",
        ));
    }
    difat.truncate(num_fat_sectors.min(difat.len()));
    let mut fat = Vec::new();
    for fat_sector_id in difat {
        let Some(data) = sector(bytes, sector_size, fat_sector_id) else {
            diagnostics.push(partial(
                "outlook.msg.cfb.fat_sector_invalid",
                format!("FAT sector {fat_sector_id} lies outside the input"),
            ));
            continue;
        };
        fat.extend(
            data.chunks_exact(4)
                .map(|chunk| u32::from_le_bytes(chunk.try_into().expect("four-byte FAT entry"))),
        );
    }
    if fat.is_empty() {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "CFB has no readable FAT",
        )));
    }
    let directory_bytes = read_regular_chain(
        bytes,
        sector_size,
        first_directory_sector,
        None,
        &fat,
        max_chain_sectors,
        "directory",
        &mut diagnostics,
    );
    if directory_bytes.len() < 128 {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "CFB directory stream is unavailable",
        )));
    }
    let records = parse_directory(&directory_bytes, major_version, &mut diagnostics);
    let Some(root_id) = records
        .iter()
        .position(|record| record.kind == Some(CfbEntryKind::Root))
    else {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "CFB root directory entry is missing",
        )));
    };
    let root_record = &records[root_id];
    let root_mini_stream = read_regular_chain(
        bytes,
        sector_size,
        root_record.start_sector,
        Some(root_record.stream_size),
        &fat,
        max_chain_sectors,
        "root mini stream",
        &mut diagnostics,
    );
    let minifat_bytes = read_regular_chain(
        bytes,
        sector_size,
        first_minifat_sector,
        Some((num_minifat_sectors.saturating_mul(sector_size)) as u64),
        &fat,
        max_chain_sectors,
        "mini FAT",
        &mut diagnostics,
    );
    let minifat = minifat_bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().expect("four-byte mini FAT entry")))
        .collect::<Vec<_>>();
    let mut paths = vec![None; records.len()];
    paths[root_id] = Some((String::new(), String::new()));
    let mut visited = BTreeSet::new();
    assign_sibling_tree(
        root_record.child,
        "",
        &records,
        &mut paths,
        &mut visited,
        max_chain_sectors,
        &mut diagnostics,
    );
    for (id, record) in records.iter().enumerate() {
        if record.kind.is_some() && paths[id].is_none() {
            let path = format!("__orphan_{id}/{}", sanitize_name(&record.name));
            paths[id] = Some((path, format!("__orphan_{id}")));
            diagnostics.push(partial(
                "outlook.msg.cfb.orphan_directory_entry",
                format!("directory entry {id} is not reachable from the root tree"),
            ));
        }
    }
    let mut entries = Vec::new();
    for (id, record) in records.iter().enumerate() {
        let Some(kind) = record.kind else { continue };
        let Some((path, parent_path)) = paths[id].clone() else {
            continue;
        };
        let data = if kind == CfbEntryKind::Stream {
            if record.stream_size == 0 {
                Vec::new()
            } else if record.stream_size < u64::from(mini_stream_cutoff) {
                read_mini_chain(
                    &root_mini_stream,
                    mini_sector_size,
                    record.start_sector,
                    record.stream_size,
                    &minifat,
                    max_chain_sectors,
                    &path,
                    &mut diagnostics,
                )
            } else {
                read_regular_chain(
                    bytes,
                    sector_size,
                    record.start_sector,
                    Some(record.stream_size),
                    &fat,
                    max_chain_sectors,
                    &path,
                    &mut diagnostics,
                )
            }
        } else {
            Vec::new()
        };
        entries.push(CfbEntry {
            id,
            name: record.name.clone(),
            path,
            parent_path,
            kind,
            clsid: record.clsid.clone(),
            data,
        });
    }
    entries.sort_by_key(|entry| entry.id);
    let storage_count = entries
        .iter()
        .filter(|entry| entry.kind == CfbEntryKind::Storage)
        .count();
    let stream_count = entries
        .iter()
        .filter(|entry| entry.kind == CfbEntryKind::Stream)
        .count();
    Ok(CfbFile {
        metadata: MsgCompoundFile {
            major_version,
            minor_version,
            sector_size,
            mini_sector_size,
            mini_stream_cutoff,
            directory_entries: records
                .iter()
                .filter(|record| record.kind.is_some())
                .count(),
            storage_count,
            stream_count,
            root_clsid: root_record.clsid.clone(),
        },
        entries,
        diagnostics,
    })
}

fn read_difat_chain(
    bytes: &[u8],
    sector_size: usize,
    first_sector: u32,
    declared_sectors: usize,
    max_chain_sectors: usize,
    difat: &mut Vec<u32>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut current = first_sector;
    let mut visited = BTreeSet::new();
    let limit = declared_sectors.min(max_chain_sectors);
    for _ in 0..limit {
        if current >= DIFAT_SECTOR || !visited.insert(current) {
            if current != END_OF_CHAIN && current != FREE_SECTOR {
                diagnostics.push(partial(
                    "outlook.msg.cfb.difat_cycle",
                    "DIFAT chain contains a cycle or reserved sector ID",
                ));
            }
            return;
        }
        let Some(data) = sector(bytes, sector_size, current) else {
            diagnostics.push(partial(
                "outlook.msg.cfb.difat_truncated",
                format!("DIFAT sector {current} lies outside the input"),
            ));
            return;
        };
        let entries = sector_size / 4;
        for chunk in data[..(entries - 1) * 4].chunks_exact(4) {
            let value = u32::from_le_bytes(chunk.try_into().expect("four-byte DIFAT entry"));
            if value < DIFAT_SECTOR {
                difat.push(value);
            }
        }
        current = u32::from_le_bytes(
            data[(entries - 1) * 4..entries * 4]
                .try_into()
                .expect("DIFAT chain pointer"),
        );
        if current == END_OF_CHAIN {
            return;
        }
    }
    if declared_sectors > max_chain_sectors {
        diagnostics.push(partial(
            "outlook.msg.cfb.chain_limit",
            "DIFAT chain exceeded max_chain_sectors",
        ));
    } else if declared_sectors > 0 && current != END_OF_CHAIN {
        diagnostics.push(partial(
            "outlook.msg.cfb.difat_count_mismatch",
            "DIFAT chain did not terminate at the declared sector count",
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn read_regular_chain(
    bytes: &[u8],
    sector_size: usize,
    start_sector: u32,
    declared_size: Option<u64>,
    fat: &[u32],
    max_chain_sectors: usize,
    label: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<u8> {
    if start_sector == END_OF_CHAIN || start_sector == FREE_SECTOR {
        return Vec::new();
    }
    let mut output = Vec::new();
    let mut current = start_sector;
    let mut visited = BTreeSet::new();
    for _ in 0..max_chain_sectors {
        if current >= DIFAT_SECTOR || !visited.insert(current) {
            diagnostics.push(partial(
                "outlook.msg.cfb.chain_cycle",
                format!("{label} chain contains a cycle or reserved sector ID"),
            ));
            break;
        }
        let Some(data) = sector(bytes, sector_size, current) else {
            diagnostics.push(partial(
                "outlook.msg.cfb.stream_truncated",
                format!("{label} sector {current} lies outside the input"),
            ));
            break;
        };
        output.extend_from_slice(data);
        let Some(next) = fat.get(current as usize).copied() else {
            diagnostics.push(partial(
                "outlook.msg.cfb.fat_entry_missing",
                format!("{label} has no FAT entry for sector {current}"),
            ));
            break;
        };
        if next == END_OF_CHAIN {
            break;
        }
        current = next;
    }
    if visited.len() == max_chain_sectors && current != END_OF_CHAIN {
        diagnostics.push(partial(
            "outlook.msg.cfb.chain_limit",
            format!("{label} chain exceeded max_chain_sectors"),
        ));
    }
    truncate_declared(&mut output, declared_size, label, diagnostics);
    output
}

#[allow(clippy::too_many_arguments)]
fn read_mini_chain(
    mini_stream: &[u8],
    mini_sector_size: usize,
    start_sector: u32,
    declared_size: u64,
    minifat: &[u32],
    max_chain_sectors: usize,
    label: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<u8> {
    let mut output = Vec::new();
    let mut current = start_sector;
    let mut visited = BTreeSet::new();
    for _ in 0..max_chain_sectors {
        if current >= DIFAT_SECTOR || !visited.insert(current) {
            diagnostics.push(partial(
                "outlook.msg.cfb.mini_chain_cycle",
                format!("{label} mini-sector chain contains a cycle or reserved sector ID"),
            ));
            break;
        }
        let Some(start) = (current as usize).checked_mul(mini_sector_size) else {
            break;
        };
        let Some(end) = start.checked_add(mini_sector_size) else {
            break;
        };
        let Some(data) = mini_stream.get(start..end) else {
            diagnostics.push(partial(
                "outlook.msg.cfb.mini_stream_truncated",
                format!("{label} mini-sector {current} lies outside the root mini stream"),
            ));
            break;
        };
        output.extend_from_slice(data);
        let Some(next) = minifat.get(current as usize).copied() else {
            diagnostics.push(partial(
                "outlook.msg.cfb.minifat_entry_missing",
                format!("{label} has no mini FAT entry for mini-sector {current}"),
            ));
            break;
        };
        if next == END_OF_CHAIN {
            break;
        }
        current = next;
    }
    if visited.len() == max_chain_sectors && current != END_OF_CHAIN {
        diagnostics.push(partial(
            "outlook.msg.cfb.chain_limit",
            format!("{label} mini-sector chain exceeded max_chain_sectors"),
        ));
    }
    truncate_declared(&mut output, Some(declared_size), label, diagnostics);
    output
}

fn truncate_declared(
    output: &mut Vec<u8>,
    declared_size: Option<u64>,
    label: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(declared) = declared_size.and_then(|value| usize::try_from(value).ok()) else {
        return;
    };
    if output.len() < declared {
        diagnostics.push(partial(
            "outlook.msg.cfb.stream_size_mismatch",
            format!(
                "{label} declares {declared} bytes but only {} are readable",
                output.len()
            ),
        ));
    } else {
        output.truncate(declared);
    }
}

fn sector(bytes: &[u8], sector_size: usize, sector_id: u32) -> Option<&[u8]> {
    let start = (sector_id as usize)
        .checked_add(1)?
        .checked_mul(sector_size)?;
    bytes.get(start..start.checked_add(sector_size)?)
}

fn parse_directory(
    bytes: &[u8],
    major_version: u16,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<DirectoryRecord> {
    let mut records = Vec::new();
    for (index, entry) in bytes.chunks_exact(128).enumerate() {
        let kind = match entry[66] {
            0 => None,
            1 => Some(CfbEntryKind::Storage),
            2 => Some(CfbEntryKind::Stream),
            5 => Some(CfbEntryKind::Root),
            other => {
                diagnostics.push(partial(
                    "outlook.msg.cfb.directory_type_unknown",
                    format!("directory entry {index} has unknown object type {other}"),
                ));
                None
            }
        };
        let declared_name_bytes = usize::from(u16::from_le_bytes([entry[64], entry[65]]));
        let valid_name_bytes = declared_name_bytes.saturating_sub(2).min(62) & !1;
        let units = entry[..valid_name_bytes]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        let name = String::from_utf16_lossy(&units);
        if kind.is_some()
            && (!(2..=64).contains(&declared_name_bytes) || declared_name_bytes % 2 != 0)
        {
            diagnostics.push(partial(
                "outlook.msg.cfb.directory_name_malformed",
                format!("directory entry {index} has invalid UTF-16 name length"),
            ));
        }
        let mut stream_size = u64::from_le_bytes(entry[120..128].try_into().expect("stream size"));
        if major_version == 3 {
            stream_size &= u64::from(u32::MAX);
        }
        records.push(DirectoryRecord {
            name,
            kind,
            left: u32::from_le_bytes(entry[68..72].try_into().expect("left sibling")),
            right: u32::from_le_bytes(entry[72..76].try_into().expect("right sibling")),
            child: u32::from_le_bytes(entry[76..80].try_into().expect("child")),
            clsid: guid(&entry[80..96]),
            start_sector: u32::from_le_bytes(entry[116..120].try_into().expect("start sector")),
            stream_size,
        });
    }
    records
}

#[allow(clippy::too_many_arguments)]
fn assign_sibling_tree(
    id: u32,
    parent_path: &str,
    records: &[DirectoryRecord],
    paths: &mut [Option<(String, String)>],
    visited: &mut BTreeSet<u32>,
    max_entries: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if id == NO_STREAM {
        return;
    }
    if visited.len() >= max_entries {
        diagnostics.push(partial(
            "outlook.msg.cfb.directory_limit",
            "directory sibling traversal exceeded max_chain_sectors",
        ));
        return;
    }
    let Some(record) = records.get(id as usize) else {
        diagnostics.push(partial(
            "outlook.msg.cfb.directory_pointer_invalid",
            format!("directory pointer {id} lies outside the directory stream"),
        ));
        return;
    };
    if !visited.insert(id) {
        diagnostics.push(partial(
            "outlook.msg.cfb.directory_cycle",
            format!("directory tree revisits entry {id}"),
        ));
        return;
    }
    assign_sibling_tree(
        record.left,
        parent_path,
        records,
        paths,
        visited,
        max_entries,
        diagnostics,
    );
    if record.kind.is_some() {
        let name = sanitize_name(&record.name);
        let path = if parent_path.is_empty() {
            name
        } else {
            format!("{parent_path}/{name}")
        };
        paths[id as usize] = Some((path.clone(), parent_path.to_string()));
        if matches!(
            record.kind,
            Some(CfbEntryKind::Storage | CfbEntryKind::Root)
        ) {
            assign_sibling_tree(
                record.child,
                &path,
                records,
                paths,
                visited,
                max_entries,
                diagnostics,
            );
        }
    }
    assign_sibling_tree(
        record.right,
        parent_path,
        records,
        paths,
        visited,
        max_entries,
        diagnostics,
    );
}

fn sanitize_name(name: &str) -> String {
    name.replace('/', "／").replace('\\', "＼")
}

fn guid(bytes: &[u8]) -> Option<String> {
    if bytes.len() != 16 || bytes.iter().all(|byte| *byte == 0) {
        return None;
    }
    Some(format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        u32::from_le_bytes(bytes[0..4].try_into().ok()?),
        u16::from_le_bytes(bytes[4..6].try_into().ok()?),
        u16::from_le_bytes(bytes[6..8].try_into().ok()?),
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, Box<Diagnostic>> {
    bytes
        .get(offset..offset + 2)
        .and_then(|value| value.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| Box::new(Diagnostic::malformed(PARSER, "CFB header is truncated")))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, Box<Diagnostic>> {
    bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| Box::new(Diagnostic::malformed(PARSER, "CFB header is truncated")))
}

fn partial(code: impl Into<String>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::warning(PARSER, code, message).partial()
}

//! Reads Xbox 360 disc images (`.iso`): finds files in them without extracting the image.
//!
//! A disc image holds the game partition's file system, XDVDFS, laid out as the ReXGlue SDK's
//! `DiscImageDevice` reads it (`src/filesystem/devices/disc_image_device.cpp`), which is what the
//! runtime mounts the image with:
//! - sectors of 2 KiB, counted from the start of the game partition. A full dump of the disc puts
//!   the partition after the video partition; an image of the game partition alone (as
//!   `extract-xiso` or `xdvdfs` make it) starts with it. The partition is found by looking for
//!   the volume descriptor at the same offsets the SDK tries;
//! - the volume descriptor, at sector 32 of the partition: the `MICROSOFT*XBOX*MEDIA` signature,
//!   then the root directory's sector and size (little-endian u32 at +20 and +24);
//! - a directory is a table of entries forming a binary tree. Each entry is 4-byte aligned:
//!   left and right child (u16, in 4-byte units from the table's start, 0 for none), the data's
//!   sector and size (u32), attributes (u8, 0x10 for a directory) and the name (length u8, then
//!   the bytes). A subdirectory's data is its own table.
//!
//! Names compare without case, as on the console. Every offset read from the image is checked
//! against the image and the table it points into, so a damaged image is reported, never read
//! out of bounds.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Bytes in a disc sector.
const SECTOR_SIZE: u64 = 2048;

/// Sector of the game partition that holds the volume descriptor.
const VOLUME_DESCRIPTOR_SECTOR: u64 = 32;

/// Signature at the start of the volume descriptor.
const MAGIC: &[u8; 20] = b"MICROSOFT*XBOX*MEDIA";

/// Where the game partition may start in an image, as the SDK's `DiscImageDevice::Verify` tries
/// them: an image of the partition alone first, then full dumps of the disc formats.
const PARTITION_OFFSETS: [u64; 5] = [0x0000_0000, 0x0000_FB20, 0x0002_0600, 0x0208_0000, 0x0FD9_0000];

/// The largest directory table read, as the SDK allows for the root.
const MAX_DIRECTORY_BYTES: u32 = 32 * 1024 * 1024;

/// Attribute bit of a directory entry that names a directory.
const ATTRIBUTE_DIRECTORY: u8 = 0x10;

/// A file or directory in the image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub sector: u32,
    pub size: u32,
    pub is_directory: bool,
}

/// An open disc image.
#[derive(Debug)]
pub struct DiscImage {
    file: File,
    /// Size of the image file, in bytes.
    len: u64,
    /// Where the game partition starts in the image, in bytes.
    partition: u64,
    root: Entry,
}

impl DiscImage {
    /// Opens `path` as a disc image. `Ok(None)` when it is a readable file but not a disc image
    /// (no volume descriptor at any of the partition offsets).
    pub fn open(path: &Path) -> io::Result<Option<Self>> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        for partition in PARTITION_OFFSETS {
            let descriptor = partition + VOLUME_DESCRIPTOR_SECTOR * SECTOR_SIZE;
            if descriptor + 28 > len {
                continue;
            }
            let mut bytes = [0u8; 28];
            file.seek(SeekFrom::Start(descriptor))?;
            file.read_exact(&mut bytes)?;
            if &bytes[..20] != MAGIC {
                continue;
            }
            let sector = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
            let size = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
            let root = Entry { name: String::new(), sector, size, is_directory: true };
            return Ok(Some(Self { file, len, partition, root }));
        }
        Ok(None)
    }

    /// Where an entry's data starts in the image, in bytes, once checked to lie inside it.
    fn data_offset(&self, entry: &Entry) -> io::Result<u64> {
        let start = self.partition + u64::from(entry.sector) * SECTOR_SIZE;
        if start + u64::from(entry.size) > self.len {
            return Err(damaged(&format!("{} lies past the end of the image", entry.name)));
        }
        Ok(start)
    }

    /// The entries of a directory, in the tree's order (by name, as images are written).
    pub fn list(&mut self, directory: &Entry) -> io::Result<Vec<Entry>> {
        if !directory.is_directory {
            return Err(damaged(&format!("{} is not a directory", directory.name)));
        }
        if directory.size == 0 {
            return Ok(Vec::new());
        }
        if directory.size > MAX_DIRECTORY_BYTES {
            return Err(damaged("a directory table is too large"));
        }
        let start = self.data_offset(directory)?;
        let mut table = vec![0u8; directory.size as usize];
        self.file.seek(SeekFrom::Start(start))?;
        self.file.read_exact(&mut table)?;
        parse_table(&table)
    }

    /// Finds an entry by its path from the root (`GearGame/CookedXbox360`, `default.xex`),
    /// comparing names without case. The root itself for an empty path.
    pub fn find(&mut self, path: &str) -> io::Result<Option<Entry>> {
        let mut current = self.root.clone();
        for part in path.split(['/', '\\']).filter(|part| !part.is_empty()) {
            if !current.is_directory {
                return Ok(None);
            }
            let Some(next) = self.list(&current)?.into_iter().find(|entry| entry.name.eq_ignore_ascii_case(part))
            else {
                return Ok(None);
            };
            current = next;
        }
        Ok(Some(current))
    }

    /// A reader of a file's data.
    pub fn open_file(&mut self, file: &Entry) -> io::Result<impl Read + '_> {
        if file.is_directory {
            return Err(damaged(&format!("{} is a directory", file.name)));
        }
        let start = self.data_offset(file)?;
        self.file.seek(SeekFrom::Start(start))?;
        Ok((&self.file).take(u64::from(file.size)))
    }
}

fn damaged(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("damaged disc image: {detail}"))
}

/// The entries of a directory table, walking its tree left, node, right. Each entry is visited
/// once at most, so a damaged table whose links form a loop still ends.
fn parse_table(table: &[u8]) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut visited = vec![false; table.len() / 4 + 1];
    // The walk keeps the nodes whose left side is done: (ordinal, left done).
    let mut stack = vec![(0usize, false)];
    while let Some((ordinal, left_done)) = stack.pop() {
        let offset = ordinal * 4;
        let header = table.get(offset..offset + 14).ok_or_else(|| damaged("an entry lies past its table"))?;
        let left = usize::from(u16::from_le_bytes([header[0], header[1]]));
        let right = usize::from(u16::from_le_bytes([header[2], header[3]]));
        if !left_done {
            if std::mem::replace(&mut visited[ordinal], true) {
                return Err(damaged("a directory table links back to itself"));
            }
            stack.push((ordinal, true));
            if left != 0 {
                stack.push((left, false));
            }
            continue;
        }
        let name_length = usize::from(header[13]);
        let name = table.get(offset + 14..offset + 14 + name_length).ok_or_else(|| damaged("a name lies past its table"))?;
        entries.push(Entry {
            name: String::from_utf8_lossy(name).into_owned(),
            sector: u32::from_le_bytes([header[4], header[5], header[6], header[7]]),
            size: u32::from_le_bytes([header[8], header[9], header[10], header[11]]),
            is_directory: header[12] & ATTRIBUTE_DIRECTORY != 0,
        });
        if right != 0 {
            stack.push((right, false));
        }
    }
    Ok(entries)
}

/// Writes small disc images for tests, in the layout described above.
#[cfg(test)]
pub mod test_image {
    use super::*;

    /// A file or directory to put in a test image.
    pub enum Node {
        File(&'static str, Vec<u8>),
        Directory(&'static str, Vec<Node>),
    }

    /// Writes `nodes` as the root of an image whose game partition starts at `partition`.
    pub fn write(path: &Path, partition: u64, nodes: &[Node]) {
        // Sector 33 onwards, after the volume descriptor: tables and data, one sector each at least.
        let mut sectors: Vec<Vec<u8>> = Vec::new();
        let (root_sector, root_size) = place_directory(nodes, &mut sectors);
        let mut image = vec![0u8; (partition + (33 + sectors.len() as u64) * SECTOR_SIZE) as usize];
        let descriptor = (partition + VOLUME_DESCRIPTOR_SECTOR * SECTOR_SIZE) as usize;
        image[descriptor..descriptor + 20].copy_from_slice(MAGIC);
        image[descriptor + 20..descriptor + 24].copy_from_slice(&root_sector.to_le_bytes());
        image[descriptor + 24..descriptor + 28].copy_from_slice(&root_size.to_le_bytes());
        for (index, data) in sectors.iter().enumerate() {
            let start = (partition + (33 + index as u64) * SECTOR_SIZE) as usize;
            image[start..start + data.len()].copy_from_slice(data);
        }
        std::fs::write(path, image).unwrap();
    }

    /// Places a directory's children and its table, returning the table's sector and size. The
    /// table is a right-leaning chain in name order (each entry's right child is the next).
    fn place_directory(nodes: &[Node], sectors: &mut Vec<Vec<u8>>) -> (u32, u32) {
        let mut placed: Vec<(&str, u32, u32, bool)> = nodes
            .iter()
            .map(|node| match node {
                Node::File(name, data) => {
                    sectors.push(data.clone());
                    let sector = 33 + sectors.len() as u32 - 1;
                    for _ in 1..data.len().div_ceil(SECTOR_SIZE as usize) {
                        sectors.push(Vec::new());
                    }
                    (*name, sector, data.len() as u32, false)
                }
                Node::Directory(name, children) => {
                    let (sector, size) = place_directory(children, sectors);
                    (*name, sector, size, true)
                }
            })
            .collect();
        placed.sort_by_key(|(name, ..)| name.to_ascii_uppercase());
        let mut table = Vec::new();
        let offsets: Vec<usize> = placed
            .iter()
            .scan(0usize, |offset, (name, ..)| {
                let here = *offset;
                *offset += (14 + name.len()).div_ceil(4) * 4;
                Some(here)
            })
            .collect();
        for (index, (name, sector, size, is_directory)) in placed.iter().enumerate() {
            let right = offsets.get(index + 1).map_or(0, |offset| (offset / 4) as u16);
            table.extend_from_slice(&0u16.to_le_bytes());
            table.extend_from_slice(&right.to_le_bytes());
            table.extend_from_slice(&sector.to_le_bytes());
            table.extend_from_slice(&size.to_le_bytes());
            table.push(if *is_directory { ATTRIBUTE_DIRECTORY } else { 0 });
            table.push(name.len() as u8);
            table.extend_from_slice(name.as_bytes());
            table.resize(table.len().div_ceil(4) * 4, 0xFF);
        }
        sectors.push(table.clone());
        let sector = 33 + sectors.len() as u32 - 1;
        for _ in 1..table.len().div_ceil(SECTOR_SIZE as usize) {
            sectors.push(Vec::new());
        }
        (sector, table.len() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::test_image::{write, Node};
    use super::*;
    use crate::test_support::TempFolder;

    fn sample() -> Vec<Node> {
        vec![
            Node::File("default.xex", b"XEX2 executable".to_vec()),
            Node::Directory(
                "GearGame",
                vec![Node::Directory(
                    "CookedXbox360",
                    vec![Node::File("Coalesced_INT.bin", vec![1; 3000]), Node::File("Coalesced_ESN.bin", vec![2])],
                )],
            ),
        ]
    }

    #[test]
    fn finds_and_reads_files_in_an_image_of_the_game_partition() {
        let dir = TempFolder::new("disc-image");
        let path = dir.join("game.iso");
        write(&path, 0, &sample());
        let mut image = DiscImage::open(&path).unwrap().unwrap();
        let xex = image.find("DEFAULT.XEX").unwrap().unwrap();
        let mut data = Vec::new();
        image.open_file(&xex).unwrap().read_to_end(&mut data).unwrap();
        assert_eq!(data, b"XEX2 executable");
        let cooked = image.find("GearGame\\CookedXbox360").unwrap().unwrap();
        let names: Vec<String> = image.list(&cooked).unwrap().into_iter().map(|entry| entry.name).collect();
        assert_eq!(names, ["Coalesced_ESN.bin", "Coalesced_INT.bin"]);
        let big = image.find("GearGame/CookedXbox360/Coalesced_INT.bin").unwrap().unwrap();
        let mut data = Vec::new();
        image.open_file(&big).unwrap().read_to_end(&mut data).unwrap();
        assert_eq!(data, vec![1; 3000]);
        assert_eq!(image.find("GearGame/missing.bin").unwrap(), None);
        assert_eq!(image.find("default.xex/inside").unwrap(), None);
    }

    #[test]
    fn finds_the_game_partition_of_a_full_disc_dump() {
        let dir = TempFolder::new("disc-image-full");
        let path = dir.join("game.iso");
        write(&path, 0x0208_0000, &sample());
        let mut image = DiscImage::open(&path).unwrap().unwrap();
        assert!(image.find("default.xex").unwrap().is_some());
    }

    #[test]
    fn a_file_that_is_not_a_disc_image_is_told_apart() {
        let dir = TempFolder::new("disc-image-other");
        let path = dir.join("notes.iso");
        std::fs::write(&path, vec![0u8; 200_000]).unwrap();
        assert!(DiscImage::open(&path).unwrap().is_none());
        std::fs::write(&path, b"short").unwrap();
        assert!(DiscImage::open(&path).unwrap().is_none());
    }

    #[test]
    fn damaged_tables_are_reported_without_reading_out_of_bounds() {
        // An entry whose right child points past the table.
        let mut table = vec![0u8; 16];
        table[2..4].copy_from_slice(&100u16.to_le_bytes());
        table[13] = 1;
        table[14] = b'a';
        assert_eq!(parse_table(&table).unwrap_err().kind(), io::ErrorKind::InvalidData);
        // Two entries that are each other's left child.
        let mut looping = vec![0u8; 32];
        looping[0..2].copy_from_slice(&4u16.to_le_bytes());
        looping[16..18].copy_from_slice(&4u16.to_le_bytes());
        assert_eq!(parse_table(&looping).unwrap_err().kind(), io::ErrorKind::InvalidData);
        // A file that claims to lie past the end of the image.
        let dir = TempFolder::new("disc-image-damaged");
        let path = dir.join("game.iso");
        write(&path, 0, &sample());
        let mut image = DiscImage::open(&path).unwrap().unwrap();
        let mut xex = image.find("default.xex").unwrap().unwrap();
        xex.size = u32::MAX;
        assert!(image.open_file(&xex).is_err());
    }
}

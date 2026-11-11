//! Finds and checks the user's copy of the game: a folder, or a disc image (`.iso`).
//!
//! A valid folder contains `default.xex`, the Xbox 360 executable the runtime recompiles. Users
//! often pick the folder one level above it (the one that holds the dump), so the immediate
//! subfolders are searched too. A disc image holds the same files in its own file system
//! (`disc_image.rs`); they are read from it in place, and the runtime mounts the image itself.
//!
//! The executable is checked in three steps:
//! 1. its XEX2 header is read, as laid out in the ReXGlue SDK (`rex/system/util/xex2_info.h`):
//!    every value is big-endian, the file starts with the `XEX2` signature, and an optional header
//!    table follows, whose execution info entry holds the title ID and media ID;
//! 2. the title ID must be the game's (from `versions.toml`), otherwise it is another game;
//! 3. the SHA-256 of the whole executable must match a known build, because recompiled code and
//!    patches only fit the exact executable they were made from.
//!
//! For a build the project does not know, the result also carries the SHA-256 of the executable's
//! image (every byte after the header), so the player can report the edition. Two editions whose
//! image hashes match differ only in the header (signature, media ID, region) and run the same
//! code (see the comment at the end of `versions.toml`).
//!
//! The game languages present are found from the engine's per-language config bundles,
//! `GearGame/CookedXbox360/Coalesced_<LANG>.bin` (INT, FRA, RUS, ...).

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::disc_image::DiscImage;
use crate::versions::Registry;

/// Name of the game executable inside the game folder.
pub const XEX_FILE_NAME: &str = "default.xex";

/// Folder, relative to the game folder, that holds the engine's cooked data and config bundles.
const COOKED_DIR: &str = "GearGame/CookedXbox360";

/// Signature at offset 0 of every XEX2 file ("XEX2" in ASCII).
const XEX2_MAGIC: u32 = 0x5845_5832;

/// Key of the optional header that holds the execution info (title ID, media ID, ...).
const EXECUTION_INFO_KEY: u32 = 0x0004_0006;

/// Offset of the field that holds where the image starts, that is, the size of the header.
const IMAGE_OFFSET_FIELD: usize = 0x08;

/// Offset of the optional header table, right after the fixed part of the XEX2 header.
const OPT_HEADER_TABLE_OFFSET: usize = 0x18;

/// Upper bound for the bytes read from the executable. The headers of a game executable take a
/// few kilobytes; the cap keeps a corrupt size field from reading a whole multi-megabyte file.
const MAX_HEADER_BYTES: u64 = 1024 * 1024;

/// Identity of a game build, read from its executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XexInfo {
    pub title_id: u32,
    pub media_id: u32,
    /// Where the image (the encrypted, compressed code and data) starts; everything before it is
    /// the header.
    pub image_offset: u32,
}

/// Why an executable could not be read as a game executable.
#[derive(Debug, PartialEq, Eq)]
pub enum XexError {
    /// The file is shorter than the structure being read.
    Truncated,
    /// The file does not start with the XEX2 signature.
    BadMagic,
    /// The header has no execution info entry, so the build cannot be identified.
    NoExecutionInfo,
}

/// Result of checking a folder or disc image, sent to the interface as JSON
/// (`{"status": "ok", ...}`).
///
/// Every variant carries `folder` so the interface can always show which path the result is about.
/// `folder` is where `default.xex` was found (possibly a subfolder of the one picked, or the disc
/// image itself), or the path that was checked when it was not found. IDs are uppercase hex, as the Xbox tools show them.
/// `languages` lists the game's language codes found in the folder (for example `["INT", "FRA"]`).
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum FolderCheck {
    /// A known build of the game: it can be played.
    #[serde(rename_all = "camelCase")]
    Ok {
        folder: String,
        title_id: String,
        media_id: String,
        version_id: String,
        version_name: String,
        languages: Vec<String>,
    },
    /// The game, but a build the project has no manifest for yet. The two hashes are what the
    /// player reports so the edition can be added: `xex_sha256` of the whole executable (the key
    /// in `versions.toml`) and `image_sha256` of the bytes after its header.
    #[serde(rename_all = "camelCase")]
    UnsupportedVersion {
        folder: String,
        title_id: String,
        media_id: String,
        languages: Vec<String>,
        xex_sha256: String,
        image_sha256: String,
    },
    /// A valid Xbox 360 executable of another game.
    #[serde(rename_all = "camelCase")]
    WrongGame { folder: String, title_id: String },
    /// The path does not exist, or is neither a folder nor a file.
    NotADirectory { folder: String },
    /// A file that is not an Xbox 360 disc image, or a damaged one.
    NotADiscImage { folder: String },
    /// A disc image without `default.xex` at its root.
    MissingXexInImage { folder: String },
    /// Neither the folder nor its immediate subfolders contain `default.xex`.
    MissingXex { folder: String },
    /// `default.xex` exists but is not a readable Xbox 360 executable.
    InvalidXex { folder: String },
}

/// Reads a big-endian u32 at `offset`, or fails when the slice is too short.
fn read_be_u32(bytes: &[u8], offset: usize) -> Result<u32, XexError> {
    let end = offset.checked_add(4).ok_or(XexError::Truncated)?;
    let chunk = bytes.get(offset..end).ok_or(XexError::Truncated)?;
    Ok(u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
}

/// Parses the start of an XEX2 file and returns the build identity.
///
/// Layout used: magic at 0x00, image offset at 0x08, optional header count at 0x14, then `count`
/// entries of (key, value) at 0x18. For the execution info key the value is an offset from the start of the
/// file to a structure whose media ID is at +0x0 and title ID at +0xC.
pub fn parse_xex_header(bytes: &[u8]) -> Result<XexInfo, XexError> {
    if read_be_u32(bytes, 0)? != XEX2_MAGIC {
        return Err(XexError::BadMagic);
    }

    let image_offset = read_be_u32(bytes, IMAGE_OFFSET_FIELD)?;
    let header_count = read_be_u32(bytes, 0x14)? as usize;
    for index in 0..header_count {
        let entry = OPT_HEADER_TABLE_OFFSET + index * 8;
        if read_be_u32(bytes, entry)? != EXECUTION_INFO_KEY {
            continue;
        }
        let info = read_be_u32(bytes, entry + 4)? as usize;
        return Ok(XexInfo {
            media_id: read_be_u32(bytes, info)?,
            title_id: read_be_u32(bytes, info + 0xC)?,
            image_offset,
        });
    }
    Err(XexError::NoExecutionInfo)
}

/// Reads the header bytes of an executable on disk and parses them.
fn read_xex_info(path: &Path) -> Option<XexInfo> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_HEADER_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    parse_xex_header(&bytes).ok()
}

/// SHA-256 digests of an executable, as lowercase hex.
#[derive(Debug, PartialEq, Eq)]
struct XexHashes {
    /// The whole file: what `versions.toml` identifies a build by.
    whole: String,
    /// The bytes from the image offset on, leaving out the header.
    image: String,
}

/// A writer that feeds every byte to the whole-file hasher and the bytes from `image_offset` on to
/// the image hasher, so one read of the executable gives both digests.
struct XexHasher {
    whole: Sha256,
    image: Sha256,
    /// Header bytes still to skip before the image hasher starts.
    header_left: u64,
}

impl io::Write for XexHasher {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.whole.update(buf);
        let skipped = buf.len().min(usize::try_from(self.header_left).unwrap_or(usize::MAX));
        self.header_left -= skipped as u64;
        self.image.update(&buf[skipped..]);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Hashes an executable read from `reader`, whose image starts at `image_offset`. Streams it, so
/// its size does not matter.
fn hash_xex(mut reader: impl Read, image_offset: u32) -> io::Result<XexHashes> {
    let mut hasher = XexHasher {
        whole: Sha256::new(),
        image: Sha256::new(),
        header_left: u64::from(image_offset),
    };
    io::copy(&mut reader, &mut hasher)?;
    Ok(XexHashes {
        whole: hex(&hasher.whole.finalize()),
        image: hex(&hasher.image.finalize()),
    })
}

/// Bytes as lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Language codes of the `Coalesced_<LANG>.bin` bundles in the game folder, sorted. Empty when
/// the folder has no cooked data (for example a folder with only the executable).
fn find_languages(game_folder: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(game_folder.join(COOKED_DIR)) else {
        return Vec::new();
    };
    languages_of(entries.filter_map(Result::ok).filter_map(|entry| entry.file_name().into_string().ok()))
}

/// Language codes of the `Coalesced_<LANG>.bin` names among `names`, uppercase and sorted.
fn languages_of(names: impl Iterator<Item = String>) -> Vec<String> {
    let mut languages: Vec<String> = names
        .filter_map(|name| {
            let code = name.strip_prefix("Coalesced_")?.strip_suffix(".bin")?;
            Some(code.to_ascii_uppercase())
        })
        .collect();
    languages.sort();
    languages
}

/// Returns the folder that holds `default.xex`: `dir` itself, or else the first immediate
/// subfolder (in name order, so the choice is stable) that does.
fn find_xex_folder(dir: &Path) -> Option<PathBuf> {
    if dir.join(XEX_FILE_NAME).is_file() {
        return Some(dir.to_path_buf());
    }
    let mut subfolders: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    subfolders.sort();
    subfolders
        .into_iter()
        .find(|path| path.join(XEX_FILE_NAME).is_file())
}

/// Checks a folder or disc image picked by the user (or remembered from a previous run) against
/// the registry of known builds.
pub fn check_folder(dir: &Path, registry: &Registry) -> FolderCheck {
    let display = |path: &Path| path.to_string_lossy().into_owned();
    if dir.is_file() {
        return check_disc_image(dir, registry);
    }
    if !dir.is_dir() {
        return FolderCheck::NotADirectory { folder: display(dir) };
    }
    let Some(game_folder) = find_xex_folder(dir) else {
        return FolderCheck::MissingXex { folder: display(dir) };
    };
    let xex_path = game_folder.join(XEX_FILE_NAME);
    let languages = find_languages(&game_folder);
    let hash = |image_offset| hash_xex(File::open(&xex_path)?, image_offset);
    identify(display(&game_folder), read_xex_info(&xex_path), languages, hash, registry)
}

/// Checks a disc image: the same checks as a folder's, on the files read from the image.
fn check_disc_image(path: &Path, registry: &Registry) -> FolderCheck {
    let folder = path.to_string_lossy().into_owned();
    let mut image = match DiscImage::open(path) {
        Ok(Some(image)) => image,
        Ok(None) | Err(_) => return FolderCheck::NotADiscImage { folder },
    };
    let xex = match image.find(XEX_FILE_NAME) {
        Ok(Some(xex)) if !xex.is_directory => xex,
        Ok(_) => return FolderCheck::MissingXexInImage { folder },
        Err(_) => return FolderCheck::NotADiscImage { folder },
    };
    let info = image.open_file(&xex).ok().and_then(|reader| {
        let mut bytes = Vec::new();
        reader.take(MAX_HEADER_BYTES).read_to_end(&mut bytes).ok()?;
        parse_xex_header(&bytes).ok()
    });
    let cooked = match image.find(COOKED_DIR) {
        Ok(Some(cooked)) if cooked.is_directory => image.list(&cooked).unwrap_or_default(),
        _ => Vec::new(),
    };
    let languages = languages_of(cooked.into_iter().map(|entry| entry.name));
    let hash = |image_offset| hash_xex(image.open_file(&xex)?, image_offset);
    identify(folder, info, languages, hash, registry)
}

/// Copies `default.xex` out of a disc image to `destination`, for the code generator, which reads
/// the executable from a file.
pub fn copy_xex_from_image(image: &Path, destination: &Path) -> io::Result<()> {
    let mut image = DiscImage::open(image)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an Xbox 360 disc image"))?;
    let xex = image
        .find(XEX_FILE_NAME)?
        .filter(|xex| !xex.is_directory)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the disc image has no default.xex"))?;
    io::copy(&mut image.open_file(&xex)?, &mut File::create(destination)?)?;
    Ok(())
}

/// The checks after `default.xex` was found, the same for a folder and a disc image: its header
/// (`info`), the title ID, then the hashes (`hash`, given the image offset and read only for the
/// game's own title ID, since it reads the whole executable) against the known builds.
fn identify(
    folder: String,
    info: Option<XexInfo>,
    languages: Vec<String>,
    hash: impl FnOnce(u32) -> io::Result<XexHashes>,
    registry: &Registry,
) -> FolderCheck {
    let Some(info) = info else {
        return FolderCheck::InvalidXex { folder };
    };

    let title_id = format!("{:08X}", info.title_id);
    if !title_id.eq_ignore_ascii_case(&registry.title_id) {
        return FolderCheck::WrongGame { folder, title_id };
    }

    let media_id = format!("{:08X}", info.media_id);
    let Ok(hashes) = hash(info.image_offset) else {
        return FolderCheck::InvalidXex { folder };
    };
    match registry.find_by_hash(&hashes.whole) {
        Some(version) => FolderCheck::Ok {
            folder,
            title_id,
            media_id,
            version_id: version.id.clone(),
            version_name: version.name.clone(),
            languages,
        },
        None => FolderCheck::UnsupportedVersion {
            folder,
            title_id,
            media_id,
            languages,
            xex_sha256: hashes.whole,
            image_sha256: hashes.image,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    const TEST_TITLE_ID: u32 = 0x4D53_08AB;

    /// Builds a minimal XEX2 header with one execution info entry pointing at offset 0x40, and an
    /// image from 0x50 on.
    fn sample_header(title_id: u32, media_id: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x60];
        bytes[0..4].copy_from_slice(&XEX2_MAGIC.to_be_bytes());
        bytes[0x08..0x0C].copy_from_slice(&0x50u32.to_be_bytes());
        bytes[0x14..0x18].copy_from_slice(&2u32.to_be_bytes());
        // First entry: an unrelated header, which must be skipped.
        bytes[0x18..0x1C].copy_from_slice(&0x0001_0100u32.to_be_bytes());
        // Second entry: execution info at offset 0x40.
        bytes[0x20..0x24].copy_from_slice(&EXECUTION_INFO_KEY.to_be_bytes());
        bytes[0x24..0x28].copy_from_slice(&0x40u32.to_be_bytes());
        bytes[0x40..0x44].copy_from_slice(&media_id.to_be_bytes());
        bytes[0x4C..0x50].copy_from_slice(&title_id.to_be_bytes());
        bytes[0x50..].fill(0xA5);
        bytes
    }

    /// SHA-256 of `bytes` as lowercase hex.
    fn sha256_hex(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    /// A registry that knows exactly one executable: `xex`.
    fn registry_for(xex: &[u8]) -> Registry {
        let hash = sha256_hex(xex);
        Registry::parse(&format!(
            r#"
            title_id = "4D5308AB"
            [[versions]]
            id = "test"
            name = "Test"
            media_id = "00000001"
            xex_sha256 = "{hash}"
            manifest = "runtime/azura_00000001.toml"
            "#
        ))
        .unwrap()
    }

    /// Creates an empty folder for one test, removed when the test ends.
    fn temp_folder(name: &str) -> TempFolder {
        TempFolder::new(name)
    }

    #[test]
    fn reads_title_and_media_id() {
        let info = parse_xex_header(&sample_header(0x1234_5678, 0x9ABC_DEF0)).unwrap();
        assert_eq!(info, XexInfo { title_id: 0x1234_5678, media_id: 0x9ABC_DEF0, image_offset: 0x50 });
    }

    #[test]
    fn rejects_files_without_the_signature() {
        let mut bytes = sample_header(1, 2);
        bytes[0] = b'M';
        assert_eq!(parse_xex_header(&bytes), Err(XexError::BadMagic));
    }

    #[test]
    fn rejects_truncated_files_without_panicking() {
        let bytes = sample_header(1, 2);
        assert_eq!(parse_xex_header(&bytes[..0x30]), Err(XexError::Truncated));
        assert_eq!(parse_xex_header(&[]), Err(XexError::Truncated));
    }

    #[test]
    fn reports_missing_execution_info() {
        let mut bytes = sample_header(1, 2);
        bytes[0x14..0x18].copy_from_slice(&1u32.to_be_bytes());
        assert_eq!(parse_xex_header(&bytes), Err(XexError::NoExecutionInfo));
    }

    #[test]
    fn recognises_a_known_build_and_its_languages() {
        let dir = temp_folder("known");
        let xex = sample_header(TEST_TITLE_ID, 0x0000_0001);
        std::fs::write(dir.join(XEX_FILE_NAME), &xex).unwrap();
        let cooked = dir.join(COOKED_DIR);
        std::fs::create_dir_all(&cooked).unwrap();
        for name in ["Coalesced_RUS.bin", "Coalesced_INT.bin", "Startup.xxx"] {
            std::fs::write(cooked.join(name), b"").unwrap();
        }
        assert_eq!(
            check_folder(&dir, &registry_for(&xex)),
            FolderCheck::Ok {
                folder: dir.to_string_lossy().into_owned(),
                title_id: "4D5308AB".into(),
                media_id: "00000001".into(),
                version_id: "test".into(),
                version_name: "Test".into(),
                languages: vec!["INT".into(), "RUS".into()],
            }
        );
    }

    #[test]
    fn recognises_a_known_build_in_a_disc_image() {
        use crate::disc_image::test_image::{write, Node};
        let dir = temp_folder("known-image");
        let xex = sample_header(TEST_TITLE_ID, 0x0000_0001);
        let image = dir.join("Gears of War 3.iso");
        write(&image, 0, &[
            Node::File("default.xex", xex.clone()),
            Node::Directory(
                "GearGame",
                vec![Node::Directory(
                    "CookedXbox360",
                    vec![Node::File("Coalesced_RUS.bin", vec![1]), Node::File("Coalesced_INT.bin", vec![1])],
                )],
            ),
        ]);
        assert_eq!(
            check_folder(&image, &registry_for(&xex)),
            FolderCheck::Ok {
                folder: image.to_string_lossy().into_owned(),
                title_id: "4D5308AB".into(),
                media_id: "00000001".into(),
                version_id: "test".into(),
                version_name: "Test".into(),
                languages: vec!["INT".into(), "RUS".into()],
            }
        );
        let copied = dir.join("default.xex");
        copy_xex_from_image(&image, &copied).unwrap();
        assert_eq!(std::fs::read(&copied).unwrap(), xex);
    }

    #[test]
    fn reports_a_file_that_is_not_a_game_image() {
        use crate::disc_image::test_image::{write, Node};
        let dir = temp_folder("bad-image");
        let other = dir.join("notes.txt");
        std::fs::write(&other, b"not a disc").unwrap();
        let folder = |path: &Path| path.to_string_lossy().into_owned();
        assert_eq!(check_folder(&other, &registry_for(b"")), FolderCheck::NotADiscImage { folder: folder(&other) });
        let without_xex = dir.join("other.iso");
        write(&without_xex, 0, &[Node::File("readme.txt", b"hi".to_vec())]);
        assert_eq!(
            check_folder(&without_xex, &registry_for(b"")),
            FolderCheck::MissingXexInImage { folder: folder(&without_xex) }
        );
        assert!(copy_xex_from_image(&without_xex, &dir.join("default.xex")).is_err());
    }

    #[test]
    fn an_unknown_build_of_the_game_is_unsupported() {
        let dir = temp_folder("unknown");
        let xex = sample_header(TEST_TITLE_ID, 0x0000_0002);
        std::fs::write(dir.join(XEX_FILE_NAME), &xex).unwrap();
        let registry = registry_for(b"some other executable");
        assert_eq!(
            check_folder(&dir, &registry),
            FolderCheck::UnsupportedVersion {
                folder: dir.to_string_lossy().into_owned(),
                title_id: "4D5308AB".into(),
                media_id: "00000002".into(),
                languages: vec![],
                xex_sha256: sha256_hex(&xex),
                image_sha256: sha256_hex(&xex[0x50..]),
            }
        );
    }

    #[test]
    fn an_unknown_build_in_a_disc_image_reports_its_hashes() {
        use crate::disc_image::test_image::{write, Node};
        let dir = temp_folder("unknown-image");
        let xex = sample_header(TEST_TITLE_ID, 0x52AC_485A);
        let image = dir.join("Gears of War 3.iso");
        write(&image, 0, &[Node::File("default.xex", xex.clone())]);
        assert_eq!(
            check_folder(&image, &registry_for(b"some other executable")),
            FolderCheck::UnsupportedVersion {
                folder: image.to_string_lossy().into_owned(),
                title_id: "4D5308AB".into(),
                media_id: "52AC485A".into(),
                languages: vec![],
                xex_sha256: sha256_hex(&xex),
                image_sha256: sha256_hex(&xex[0x50..]),
            }
        );
    }

    #[test]
    fn editions_that_differ_only_in_the_header_share_the_image_hash() {
        // A header the size of the game's (0x4000) and an image longer than one copy buffer, so
        // the header ends inside a write.
        let mut first = vec![0u8; 0x4000 + 0x5000];
        first.iter_mut().enumerate().for_each(|(index, byte)| *byte = index as u8);
        let mut second = first.clone();
        second[0x100] ^= 0xFF;
        let first_hashes = hash_xex(first.as_slice(), 0x4000).unwrap();
        let second_hashes = hash_xex(second.as_slice(), 0x4000).unwrap();
        assert_eq!(first_hashes.whole, sha256_hex(&first));
        assert_eq!(first_hashes.image, sha256_hex(&first[0x4000..]));
        assert_ne!(first_hashes.whole, second_hashes.whole);
        assert_eq!(first_hashes.image, second_hashes.image);
    }

    #[test]
    fn an_image_offset_past_the_end_hashes_an_empty_image() {
        let hashes = hash_xex(&b"short"[..], 0x4000).unwrap();
        assert_eq!(hashes.whole, sha256_hex(b"short"));
        assert_eq!(hashes.image, sha256_hex(b""));
    }

    #[test]
    fn another_game_is_rejected_by_title_id() {
        let dir = temp_folder("other-game");
        let xex = sample_header(0x1234_5678, 1);
        std::fs::write(dir.join(XEX_FILE_NAME), &xex).unwrap();
        // Even a hash listed in the registry does not make another title acceptable.
        assert_eq!(
            check_folder(&dir, &registry_for(&xex)),
            FolderCheck::WrongGame {
                folder: dir.to_string_lossy().into_owned(),
                title_id: "12345678".into(),
            }
        );
    }

    #[test]
    fn finds_the_executable_in_a_subfolder() {
        let dir = temp_folder("nested");
        let game = dir.join("Gears of War 3");
        std::fs::create_dir(&game).unwrap();
        let xex = sample_header(TEST_TITLE_ID, 1);
        std::fs::write(game.join(XEX_FILE_NAME), &xex).unwrap();
        let FolderCheck::Ok { folder, .. } = check_folder(&dir, &registry_for(&xex)) else {
            panic!("expected the nested executable to be found");
        };
        assert_eq!(folder, game.to_string_lossy());
    }

    #[test]
    fn reports_each_kind_of_bad_folder_with_its_path() {
        let dir = temp_folder("bad");
        let registry = registry_for(b"");
        let folder = dir.to_string_lossy().into_owned();
        let missing = dir.join("missing");
        assert_eq!(
            check_folder(&missing, &registry),
            FolderCheck::NotADirectory { folder: missing.to_string_lossy().into_owned() }
        );
        assert_eq!(check_folder(&dir, &registry), FolderCheck::MissingXex { folder: folder.clone() });
        std::fs::write(dir.join(XEX_FILE_NAME), b"not an executable").unwrap();
        assert_eq!(check_folder(&dir, &registry), FolderCheck::InvalidXex { folder });
    }
}

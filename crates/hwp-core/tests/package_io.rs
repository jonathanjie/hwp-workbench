use hwp_core::errors::{MAX_EXPANDED, MAX_PART};
use hwp_core::package_io::ZipPackage;
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use zip::{write::SimpleFileOptions, CompressionMethod, DateTime, ZipArchive, ZipWriter};

fn package(payload: &[u8]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.set_raw_comment(b"comment".to_vec().into()).unwrap();
    for (name, bytes, method) in [
        (
            "mimetype",
            b"application/hwp+zip".as_slice(),
            CompressionMethod::Stored,
        ),
        ("BinData/data.bin", payload, CompressionMethod::Deflated),
    ] {
        writer
            .start_file(
                name,
                SimpleFileOptions::default()
                    .compression_method(method)
                    .last_modified_time(DateTime::from_date_and_time(2026, 1, 1, 0, 0, 0).unwrap()),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
fn fields(bytes: &[u8]) -> (usize, usize) {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let member = archive.by_index_raw(1).unwrap();
    (
        member.header_start() as usize,
        member.central_header_start() as usize,
    )
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn understated(mut bytes: Vec<u8>) -> Vec<u8> {
    let (local, central) = fields(&bytes);
    let mut crc = flate2::Crc::new();
    crc.update(b"A");
    put32(&mut bytes, local + 14, crc.sum());
    put32(&mut bytes, central + 16, crc.sum());
    put32(&mut bytes, local + 22, 1);
    put32(&mut bytes, central + 24, 1);
    bytes
}
#[test]
fn test_forged_size_cannot_hide_expansion() {
    let bytes = understated(package(&vec![b'A'; 33 * 1024 * 1024]));
    assert_eq!(
        ZipPackage::new(&bytes)
            .unwrap()
            .read_member(1, 0, MAX_PART, MAX_EXPANDED)
            .unwrap_err()
            .code,
        "LIMIT"
    );
}
#[test]
fn test_actual_total_budget_is_enforced_with_understated_member() {
    let bytes = understated(package(&vec![b'A'; 8192]));
    assert_eq!(
        ZipPackage::new(&bytes)
            .unwrap()
            .read_member(1, 100, MAX_PART, 4096)
            .unwrap_err()
            .code,
        "LIMIT"
    );
}
#[test]
fn test_crc_and_deflate_completion_are_checked() {
    let bytes = package(&vec![b'A'; 8192]);
    let (local, central) = fields(&bytes);
    for crc in [true, false] {
        let mut corrupt = bytes.clone();
        if crc {
            put32(&mut corrupt, local + 14, 0);
            put32(&mut corrupt, central + 16, 0);
        } else {
            let size =
                u32::from_le_bytes(bytes[central + 20..central + 24].try_into().unwrap()) - 1;
            put32(&mut corrupt, local + 18, size);
            put32(&mut corrupt, central + 20, size);
        }
        assert_eq!(
            ZipPackage::new(&corrupt)
                .unwrap()
                .read_member(1, 0, MAX_PART, MAX_EXPANDED)
                .unwrap_err()
                .code,
            "BAD_ZIP"
        );
    }
}
#[test]
fn test_inflated_length_must_match_header_even_below_limit() {
    let bytes = understated(package(&vec![b'A'; 1000]));
    assert_eq!(
        ZipPackage::new(&bytes)
            .unwrap()
            .read_member(1, 0, MAX_PART, MAX_EXPANDED)
            .unwrap_err()
            .code,
        "BAD_ZIP"
    );
}
#[test]
fn test_fixture_package_bytes_do_not_change_with_clock() {
    assert_eq!(package(b"authored bytes"), package(b"authored bytes"));
}
#[test]
fn rewrite_keeps_raw_unchanged_entries_and_metadata() {
    let bytes = package(b"old");
    let package = ZipPackage::new(&bytes).unwrap();
    assert_eq!(package.rewrite(&BTreeMap::new()).unwrap(), bytes);
    let output = package
        .rewrite(&BTreeMap::from([(
            "BinData/data.bin".to_owned(),
            "new 한글".as_bytes().to_vec(),
        )]))
        .unwrap();
    let mut old = ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut new = ZipArchive::new(Cursor::new(&output)).unwrap();
    assert_eq!(old.comment(), new.comment());
    for i in 0..old.len() {
        let mut a = old.by_index_raw(i).unwrap();
        let mut b = new.by_index_raw(i).unwrap();
        assert_eq!(a.name(), b.name());
        assert_eq!(a.compression(), b.compression());
        assert_eq!(a.last_modified(), b.last_modified());
        if i == 0 {
            let mut aa = vec![];
            let mut bb = vec![];
            a.read_to_end(&mut aa).unwrap();
            b.read_to_end(&mut bb).unwrap();
            assert_eq!(aa, bb);
        }
    }
    assert_eq!(
        ZipPackage::new(&output)
            .unwrap()
            .read_member(1, 0, MAX_PART, MAX_EXPANDED)
            .unwrap(),
        "new 한글".as_bytes()
    );
}
#[test]
fn local_header_name_mismatch_and_deflate_suffix_rejected() {
    let mut bytes = package(b"test");
    let (local, _) = fields(&bytes);
    bytes[local + 30] = b'Z';
    assert_eq!(ZipPackage::new(&bytes).err().unwrap().code, "BAD_ZIP");
}
#[test]
fn duplicate_raw_inventory_is_rejected_before_zip_deduplication() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for name in ["aaa", "bbb"] {
        writer
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(b"data").unwrap();
    }
    let mut bytes = writer.finish().unwrap().into_inner();
    let mut archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let member = archive.by_index_raw(1).unwrap();
    let local = member.header_start() as usize;
    let central = member.central_header_start() as usize;
    drop(member);
    drop(archive);
    bytes[local + 30..local + 33].copy_from_slice(b"aaa");
    bytes[central + 46..central + 49].copy_from_slice(b"aaa");
    assert_eq!(ZipPackage::new(&bytes).err().unwrap().code, "UNSAFE_ZIP");
}
#[test]
fn rewrite_keeps_unknown_local_and_central_extra_fields() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for name in ["unchanged", "changed"] {
        let mut options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .into_full_options();
        options
            .add_extra_data(0x1234, b"opaque local", false)
            .unwrap();
        options
            .add_extra_data(0x1235, b"opaque central", true)
            .unwrap();
        writer.start_file(name, options).unwrap();
        writer.write_all(b"old").unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let output = ZipPackage::new(&bytes)
        .unwrap()
        .rewrite(&BTreeMap::from([("changed".into(), b"new".to_vec())]))
        .unwrap();
    let mut old = ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut new = ZipArchive::new(Cursor::new(&output)).unwrap();
    for index in 0..2 {
        let a = old.by_index_raw(index).unwrap();
        let b = new.by_index_raw(index).unwrap();
        assert_eq!(a.extra_data(), b.extra_data());
        let extra_a = a.header_start() as usize + 30 + a.name_raw().len();
        let extra_b = b.header_start() as usize + 30 + b.name_raw().len();
        let n = u16::from_le_bytes(
            bytes[a.header_start() as usize + 28..a.header_start() as usize + 30]
                .try_into()
                .unwrap(),
        ) as usize;
        assert_eq!(&bytes[extra_a..extra_a + n], &output[extra_b..extra_b + n]);
    }
}
#[test]
fn zip64_local_headers_survive_changed_and_unchanged_entries() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for name in ["a", "b"] {
        writer
            .start_file(
                name,
                SimpleFileOptions::default()
                    .large_file(true)
                    .compression_method(CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(b"old payload").unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let output = ZipPackage::new(&bytes)
        .unwrap()
        .rewrite(&BTreeMap::from([(
            "a".into(),
            b"new payload with different length".to_vec(),
        )]))
        .unwrap();
    let mut package = ZipPackage::new(&output).unwrap();
    assert_eq!(
        package.read_member(0, 0, MAX_PART, MAX_EXPANDED).unwrap(),
        b"new payload with different length"
    );
    assert_eq!(
        package.read_member(1, 0, MAX_PART, MAX_EXPANDED).unwrap(),
        b"old payload"
    );
}
#[test]
fn descriptor_and_zip64_end_records_are_updated() {
    // Build a small ZIP independently, including a data descriptor and ZIP64 end.
    let data = b"old";
    let mut crc = flate2::Crc::new();
    crc.update(data);
    let mut bytes = vec![0u8; 30];
    bytes[..4].copy_from_slice(b"PK\x03\x04");
    bytes[4..6].copy_from_slice(&45u16.to_le_bytes());
    bytes[6..8].copy_from_slice(&8u16.to_le_bytes());
    bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
    bytes.extend(b"a");
    bytes.extend(data);
    bytes.extend(b"PK\x07\x08");
    bytes.extend(crc.sum().to_le_bytes());
    bytes.extend(3u32.to_le_bytes());
    bytes.extend(3u32.to_le_bytes());
    let central_start = bytes.len();
    let mut central = vec![0u8; 46];
    central[..4].copy_from_slice(b"PK\x01\x02");
    central[4..6].copy_from_slice(&45u16.to_le_bytes());
    central[6..8].copy_from_slice(&45u16.to_le_bytes());
    central[8..10].copy_from_slice(&8u16.to_le_bytes());
    put32(&mut central, 16, crc.sum());
    put32(&mut central, 20, 3);
    put32(&mut central, 24, 3);
    central[28..30].copy_from_slice(&1u16.to_le_bytes());
    central.extend(b"a");
    let central_size = central.len();
    bytes.extend(central);
    let zip64_start = bytes.len();
    bytes.extend(b"PK\x06\x06");
    bytes.extend(44u64.to_le_bytes());
    bytes.extend(45u16.to_le_bytes());
    bytes.extend(45u16.to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(1u64.to_le_bytes());
    bytes.extend(1u64.to_le_bytes());
    bytes.extend((central_size as u64).to_le_bytes());
    bytes.extend((central_start as u64).to_le_bytes());
    bytes.extend(b"PK\x06\x07");
    bytes.extend(0u32.to_le_bytes());
    bytes.extend((zip64_start as u64).to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(b"PK\x05\x06");
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(u16::MAX.to_le_bytes());
    bytes.extend(u16::MAX.to_le_bytes());
    bytes.extend(u32::MAX.to_le_bytes());
    bytes.extend(u32::MAX.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    let output = ZipPackage::new(&bytes)
        .unwrap()
        .rewrite(&BTreeMap::from([(
            "a".into(),
            b"longer new payload".to_vec(),
        )]))
        .unwrap();
    assert_eq!(
        ZipPackage::new(&output)
            .unwrap()
            .read_member(0, 0, MAX_PART, MAX_EXPANDED)
            .unwrap(),
        b"longer new payload"
    );
    let mut standard = ZipArchive::new(Cursor::new(&output)).unwrap();
    let mut data = vec![];
    standard
        .by_index(0)
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    assert_eq!(data, b"longer new payload");
}
#[test]
fn test_zip_traversal_rejected() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("../outside", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"evil").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    assert_eq!(ZipPackage::new(&bytes).err().unwrap().code, "UNSAFE_ZIP");
}
#[test]
fn deflate_suffix_and_stored_budgets_are_checked() {
    let mut bytes = package(&vec![b'A'; 8192]);
    let (local, central) = fields(&bytes);
    let size = u32::from_le_bytes(bytes[central + 20..central + 24].try_into().unwrap());
    let first_central = ZipArchive::new(Cursor::new(&bytes))
        .unwrap()
        .central_directory_start() as usize;
    bytes.insert(first_central, 0);
    let central = central + 1;
    put32(&mut bytes, local + 18, size + 1);
    put32(&mut bytes, central + 20, size + 1);
    let footer = bytes.len() - 22 - 7;
    let offset = u32::from_le_bytes(bytes[footer + 16..footer + 20].try_into().unwrap());
    put32(&mut bytes, footer + 16, offset + 1);
    assert_eq!(
        ZipPackage::new(&bytes)
            .unwrap()
            .read_member(1, 0, MAX_PART, MAX_EXPANDED)
            .unwrap_err()
            .message,
        "Data follows the ZIP deflate stream."
    );
    let bytes = package(b"a");
    assert_eq!(
        ZipPackage::new(&bytes)
            .unwrap()
            .read_member(0, 0, 1, MAX_EXPANDED)
            .unwrap_err()
            .code,
        "LIMIT"
    );
    assert_eq!(first_central as u32, offset);
}
#[test]
fn unicode_path_aliases_keep_contract_raw_member_names() {
    let mut crc = flate2::Crc::new();
    crc.update(b"legacy");
    let mut extra = vec![1];
    extra.extend(crc.sum().to_le_bytes());
    extra.extend("한글".as_bytes());
    let mut options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .into_full_options();
    options.add_extra_data(0x1234, &extra, false).unwrap();
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("legacy", options).unwrap();
    writer.write_all(b"old").unwrap();
    let mut bytes = writer.finish().unwrap().into_inner();
    let archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let central = archive.central_directory_start() as usize;
    drop(archive);
    bytes[36..38].copy_from_slice(&0x7075u16.to_le_bytes());
    bytes[central + 52..central + 54].copy_from_slice(&0x7075u16.to_le_bytes());
    let package = ZipPackage::new(&bytes).unwrap();
    assert_eq!(package.names().collect::<Vec<_>>(), vec!["legacy"]);
    let output = package
        .rewrite(&BTreeMap::from([("legacy".into(), b"new".to_vec())]))
        .unwrap();
    let mut package = ZipPackage::new(&output).unwrap();
    assert_eq!(package.names().collect::<Vec<_>>(), vec!["legacy"]);
    assert_eq!(
        package.read_member(0, 0, MAX_PART, MAX_EXPANDED).unwrap(),
        b"new"
    );
}
#[test]
fn unsigned_descriptor_with_signature_valued_crc_is_supported() {
    let payload = [0xac, 0x0a, 0x7a, 0xd5];
    let mut crc = flate2::Crc::new();
    crc.update(&payload);
    assert_eq!(crc.sum(), 0x08074b50);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "a",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(&payload).unwrap();
    let mut bytes = writer.finish().unwrap().into_inner();
    let archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let central = archive.central_directory_start() as usize;
    drop(archive);
    let mut descriptor = vec![];
    descriptor.extend(crc.sum().to_le_bytes());
    descriptor.extend(4u32.to_le_bytes());
    descriptor.extend(4u32.to_le_bytes());
    bytes.splice(central..central, descriptor);
    bytes[6..8].copy_from_slice(&8u16.to_le_bytes());
    bytes[central + 12 + 8..central + 12 + 10].copy_from_slice(&8u16.to_le_bytes());
    let footer = bytes.len() - 22;
    put32(&mut bytes, footer + 16, (central + 12) as u32);
    let output = ZipPackage::new(&bytes)
        .unwrap()
        .rewrite(&BTreeMap::from([("a".into(), b"new".to_vec())]))
        .unwrap();
    assert_eq!(
        ZipPackage::new(&output)
            .unwrap()
            .read_member(0, 0, MAX_PART, MAX_EXPANDED)
            .unwrap(),
        b"new"
    );
}
#[test]
fn truncated_local_zip64_field_cannot_overwrite_opaque_extra() {
    let mut options = SimpleFileOptions::default()
        .large_file(true)
        .compression_method(CompressionMethod::Stored)
        .into_full_options();
    options
        .add_extra_data(0x1234, b"opaque payload retained", false)
        .unwrap();
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("a", options).unwrap();
    writer.write_all(b"old").unwrap();
    let mut bytes = writer.finish().unwrap().into_inner();
    let n = u16::from_le_bytes(bytes[28..30].try_into().unwrap()) as usize;
    let mut at = 31;
    let end = at + n;
    let mut found = false;
    while at + 4 <= end {
        let id = u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
        let length = u16::from_le_bytes(bytes[at + 2..at + 4].try_into().unwrap()) as usize;
        if id == 1 {
            bytes[at + 2..at + 4].copy_from_slice(&8u16.to_le_bytes());
            found = true;
            break;
        }
        at += 4 + length;
    }
    assert!(found);
    let package = ZipPackage::new(&bytes).unwrap();
    let before = bytes.clone();
    assert_eq!(
        package
            .rewrite(&BTreeMap::from([("a".into(), b"different bytes".to_vec())]))
            .unwrap_err()
            .code,
        "BAD_ZIP"
    );
    assert_eq!(bytes, before);
}

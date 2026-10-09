use hwp_core::cfb::{p32, p64, u32, CompoundFile, END, FAT, FREE, MAGIC};
use std::collections::BTreeMap;

// Author directory topology independently; the seed has only zero-length streams.
// Like the Python fixture, rewrite then allocates payload sectors under test.
fn fixture(streams: &[(&str, Vec<u8>)], major: u16) -> Vec<u8> {
    let ss = if major == 3 { 512 } else { 4096 };
    let mut paths = BTreeMap::new();
    for (name, _) in streams {
        paths.insert(name.to_string(), 2u8);
        let mut parent = *name;
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            paths.entry(prefix.to_owned()).or_insert(1);
            parent = prefix;
        }
    }
    let ids: BTreeMap<_, _> = paths
        .keys()
        .enumerate()
        .map(|(i, name)| (name.clone(), i + 1))
        .collect();
    let count = (paths.len() + 1) * 128;
    let mut directory = vec![0u8; count.div_ceil(ss) * ss];
    for (name, index, kind) in std::iter::once(("Root Entry".to_string(), 0, 5)).chain(
        paths
            .iter()
            .map(|(p, kind)| (p.rsplit('/').next().unwrap().to_owned(), ids[p], *kind)),
    ) {
        let at = index * 128;
        let encoded: Vec<_> = name
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        directory[at..at + encoded.len()].copy_from_slice(&encoded);
        directory[at + 64..at + 66].copy_from_slice(&(encoded.len() as u16).to_le_bytes());
        directory[at + 66] = kind;
        directory[at + 67] = 1;
        for off in [68, 72, 76] {
            p32(&mut directory, at + off, FREE).unwrap();
        }
        p32(&mut directory, at + 116, END).unwrap();
        p64(&mut directory, at + 120, 0).unwrap();
    }
    for parent in std::iter::once("".to_owned()).chain(
        paths
            .iter()
            .filter(|(_, kind)| **kind == 1)
            .map(|(name, _)| name.clone()),
    ) {
        let children: Vec<_> = paths
            .keys()
            .filter(|path| path.rsplit_once('/').map(|(p, _)| p).unwrap_or("") == parent)
            .collect();
        let parent_id = ids.get(&parent).copied().unwrap_or(0);
        if let Some(first) = children.first() {
            p32(&mut directory, parent_id * 128 + 76, ids[*first] as u32).unwrap();
        }
        for pair in children.windows(2) {
            p32(&mut directory, ids[pair[0]] * 128 + 72, ids[pair[1]] as u32).unwrap();
        }
    }
    let n = directory.len() / ss;
    let fat_id = n as u32;
    let mut header = vec![0; ss];
    header[..8].copy_from_slice(&MAGIC);
    header[26..28].copy_from_slice(&major.to_le_bytes());
    header[28..30].copy_from_slice(&0xfffeu16.to_le_bytes());
    header[30..32].copy_from_slice(&(if major == 3 { 9u16 } else { 12 }).to_le_bytes());
    header[32..34].copy_from_slice(&6u16.to_le_bytes());
    for (at, value) in [
        (40, if major == 4 { n as u32 } else { 0 }),
        (44, 1),
        (48, 0),
        (56, 4096),
        (60, END),
        (64, 0),
        (68, END),
        (72, 0),
    ] {
        p32(&mut header, at, value).unwrap();
    }
    header[76..512].fill(0xff);
    p32(&mut header, 76, fat_id).unwrap();
    let mut fat = vec![0xff; ss];
    for i in 0..n {
        p32(
            &mut fat,
            i * 4,
            if i + 1 == n { END } else { (i + 1) as u32 },
        )
        .unwrap();
    }
    p32(&mut fat, n * 4, FAT).unwrap();
    header.extend(directory);
    header.extend(fat);
    let seed = CompoundFile::new(&header).unwrap();
    let changed: BTreeMap<_, _> = streams
        .iter()
        .map(|(name, data)| (name.to_string(), data.clone()))
        .collect();
    seed.rewrite(&changed).unwrap()
}
fn base(major: u16) -> Vec<u8> {
    fixture(
        &[
            ("DocInfo", b"opaque info".to_vec()),
            (
                "BodyText/Section0",
                "마리나체인 검토 중".as_bytes().to_vec(),
            ),
            ("BinData/opaque.bin", b"IMAGE-OPAQUE".repeat(800)),
        ],
        major,
    )
}
#[test]
fn test_cfb_v4() {
    let bytes = base(4);
    let c = CompoundFile::new(&bytes).unwrap();
    let output = c
        .rewrite(&BTreeMap::from([(
            "BodyText/Section0".to_owned(),
            "마리나체인 검토 완".as_bytes().to_vec(),
        )]))
        .unwrap();
    let r = CompoundFile::new(&output).unwrap();
    assert_eq!(r.major, 4);
    assert_eq!(
        r.streams["BodyText/Section0"],
        "마리나체인 검토 완".as_bytes()
    );
    assert_eq!(
        r.streams["BinData/opaque.bin"],
        c.streams["BinData/opaque.bin"]
    );
}
#[test]
fn test_cfb_fat_cycles_rejected() {
    let mut bytes = base(3);
    let fatid = u32(&bytes, 76).unwrap() as usize;
    let dir = u32(&bytes, 48).unwrap();
    p32(&mut bytes, (fatid + 1) * 512 + dir as usize * 4, dir).unwrap();
    assert_eq!(CompoundFile::new(&bytes).unwrap_err().code, "BAD_CFB");
}
#[test]
fn test_cfb_opaque_stream_resize() {
    let bytes = base(3);
    let c = CompoundFile::new(&bytes).unwrap();
    let data = b"large".repeat(2000);
    let output = c
        .rewrite(&BTreeMap::from([("DocInfo".into(), data.clone())]))
        .unwrap();
    let r = CompoundFile::new(&output).unwrap();
    assert_eq!(r.streams["DocInfo"], data);
    for (name, old) in &c.streams {
        if name != "DocInfo" {
            assert_eq!(&r.streams[name], old);
        }
    }
}
#[test]
fn test_cfb_ministream_transition() {
    let c = CompoundFile::new(&base(3)).unwrap();
    let r = CompoundFile::new(
        &c.rewrite(&BTreeMap::from([("DocInfo".into(), vec![b'a'; 4097])]))
            .unwrap(),
    )
    .unwrap();
    let r = CompoundFile::new(
        &r.rewrite(&BTreeMap::from([("DocInfo".into(), b"tiny".to_vec())]))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(r.streams["DocInfo"], b"tiny");
}
#[test]
fn test_cfb_difat() {
    let bytes = fixture(
        &[("Large", vec![b'x'; 8_000_000]), ("Small", b"y".to_vec())],
        3,
    );
    assert!(u32(&bytes, 44).unwrap() > 109);
    let c = CompoundFile::new(&bytes).unwrap();
    assert_eq!(c.streams["Large"], vec![b'x'; 8_000_000]);
    assert_eq!(c.streams["Small"], b"y");
    let output = c
        .rewrite(&BTreeMap::from([("Small".into(), b"new".to_vec())]))
        .unwrap();
    assert_eq!(
        CompoundFile::new(&output).unwrap().streams["Large"],
        c.streams["Large"]
    );
}
#[test]
fn test_cfb_truncated() {
    let mut bytes = base(3);
    bytes.pop();
    assert_eq!(CompoundFile::new(&bytes).unwrap_err().code, "BAD_CFB");
}
#[test]
fn noop_and_same_payload_are_byte_identical() {
    let bytes = base(3);
    let c = CompoundFile::new(&bytes).unwrap();
    assert_eq!(c.rewrite(&BTreeMap::new()).unwrap(), bytes);
    assert_eq!(
        c.rewrite(&BTreeMap::from([(
            "DocInfo".into(),
            c.streams["DocInfo"].clone()
        )]))
        .unwrap(),
        bytes
    );
    assert_eq!(
        c.rewrite(&BTreeMap::from([("new".into(), b"x".to_vec())]))
            .unwrap_err()
            .code,
        "BAD_CFB"
    );
}
#[test]
fn malformed_directory_and_allocation_are_bounded() {
    for bytes in [vec![], vec![0; 512], vec![0; 513]] {
        assert_eq!(CompoundFile::new(&bytes).unwrap_err().code, "BAD_CFB");
    }
    let bytes = base(3);
    for (at, value) in [
        (44, u32::MAX),
        (64, u32::MAX),
        (72, u32::MAX),
        (48, 0x12345678),
    ] {
        let mut b = bytes.clone();
        p32(&mut b, at, value).unwrap();
        assert_eq!(CompoundFile::new(&b).unwrap_err().code, "BAD_CFB");
    }
    assert_eq!(u32(&[0], 0).unwrap_err().code, "BAD_CFB");
    assert_eq!(u32(&bytes, usize::MAX).unwrap_err().code, "BAD_CFB");
}
#[test]
fn unicode_casefold_duplicate_names_are_rejected() {
    let mut bytes = fixture(&[("Straße", b"a".to_vec()), ("STRASSX", b"b".to_vec())], 3);
    let c = CompoundFile::new(&bytes).unwrap();
    let index = c.paths["STRASSX"];
    let directory = u32(&bytes, 48).unwrap() as usize;
    let at = (directory + 1) * 512 + index * 128 + 12;
    bytes[at..at + 2].copy_from_slice(&(b'E' as u16).to_le_bytes());
    assert_eq!(CompoundFile::new(&bytes).unwrap_err().code, "BAD_CFB");
}

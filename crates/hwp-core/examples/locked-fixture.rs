//! Build the authored renderer search fixture; this is not corpus evidence.
use hwp_core::document::Document;
use hwp_core::package_io::ZipPackage;
use std::collections::BTreeMap;
use std::io::Write;

fn fixture() -> hwp_core::errors::Result<Vec<u8>> {
    let original = include_bytes!("../../../fixtures/generated/review-form.hwpx");
    let mut package = ZipPackage::new(original)?;
    let index = package
        .index_for_name("Contents/section0.xml")
        .expect("fixture section");
    let section = package.read_member(
        index,
        0,
        hwp_core::errors::MAX_PART,
        hwp_core::errors::MAX_EXPANDED,
    )?;
    let source = std::str::from_utf8(&section).expect("authored fixture is UTF-8");
    let first = source.find("<hp:t>").expect("first fixture text");
    let first_end =
        first + source[first..].find("</hp:t>").expect("first text end") + "</hp:t>".len();
    let second = first_end
        + source[first_end..]
            .find("<hp:t>")
            .expect("second fixture text");
    let second_end = second + source[second..].find("</hp:t>").expect("second text end") + 7;
    let section = format!(
        "{}<hp:t>C2 match editable</hp:t>{}<hp:fieldBegin><hp:t>C2 match locked</hp:t></hp:fieldBegin>{}",
        &source[..first], &source[first_end..second], &source[second_end..]
    );
    let bytes = package.rewrite(&BTreeMap::from([(
        "Contents/section0.xml".into(),
        section.into_bytes(),
    )]))?;
    // Assert the native reader sees both expected states before publication.
    let document = Document::new(&bytes, "locked.hwpx")?;
    assert!(document
        .nodes
        .iter()
        .any(|node| node.text == "C2 match editable" && node.editable));
    assert!(document
        .nodes
        .iter()
        .any(|node| node.text == "C2 match locked" && !node.editable));
    Ok(bytes)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let destination = args.next().ok_or("Usage: locked-fixture OUTPUT.hwpx")?;
    if args.next().is_some() {
        return Err("Usage: locked-fixture OUTPUT.hwpx".into());
    }
    let bytes = fixture()?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

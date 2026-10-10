//! Preserve the direct mapper's published XML spelling and package member order.
use hwp_convert::package::{encoded, read_package, write_package_ordered, Parts, Trees, XmlNode};
use hwp_core::{
    errors::Result,
    xml_io::{Element, QualifiedName},
};
use std::collections::BTreeMap;

fn number(name: &str) -> usize {
    name.chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}
fn generated_order(parts: &Parts, trees: &Trees, target: &str) -> Vec<String> {
    let mut result = Vec::new();
    if target == "docx" {
        let mut images: Vec<_> = parts
            .keys()
            .filter(|name| name.starts_with("word/media/"))
            .cloned()
            .collect();
        images.sort_by_key(|name| number(name));
        result.extend(images);
        result.extend(
            [
                "word/document.xml",
                "word/styles.xml",
                "word/numbering.xml",
                "word/_rels/document.xml.rels",
                "_rels/.rels",
                "[Content_Types].xml",
            ]
            .map(str::to_owned),
        );
    } else {
        result.push("mimetype".into());
        let mut sections: Vec<_> = parts
            .keys()
            .filter(|name| name.starts_with("Contents/section") && name.ends_with(".xml"))
            .cloned()
            .collect();
        sections.sort_by_key(|name| number(name));
        let image_paths: BTreeMap<_, _> = parts
            .keys()
            .filter(|name| name.starts_with("BinData/"))
            .map(|name| {
                (
                    name.rsplit('/')
                        .next()
                        .unwrap_or(name)
                        .split('.')
                        .next()
                        .unwrap_or(name),
                    name,
                )
            })
            .collect();
        for section in sections {
            if let Some(tree) = trees.get(&section) {
                for element in tree.iter() {
                    if let Some(image) = element
                        .get("binaryItemIDRef")
                        .and_then(|id| image_paths.get(id))
                    {
                        result.push((*image).clone());
                    }
                }
            }
            result.push(section);
        }
        result.extend(
            [
                "Contents/header.xml",
                "Contents/content.hpf",
                "version.xml",
                "META-INF/container.xml",
            ]
            .map(str::to_owned),
        );
    }
    result
}
fn namespace(name: &QualifiedName, namespaces: &mut BTreeMap<String, String>) {
    if let Some(uri) = &name.namespace {
        let prefix = name
            .qualified
            .split_once(':')
            .map(|(prefix, _)| prefix)
            .unwrap_or("");
        if prefix != "xml" {
            namespaces.insert(prefix.into(), uri.clone());
        }
    }
}
// Python constructs dynamic attributes before each element's fixed defaults.
fn attribute_order(name: &str) -> &'static [&'static str] {
    match name {
        "hp:tbl" => &[
            "id",
            "rowCnt",
            "colCnt",
            "cellSpacing",
            "borderFillIDRef",
            "pageBreak",
            "repeatHeader",
        ],
        "hp:sz" => &["width", "height", "widthRelTo", "heightRelTo", "protect"],
        "hp:pic" => &[
            "id",
            "zOrder",
            "numberingType",
            "textWrap",
            "textFlow",
            "reverse",
        ],
        "hc:img" => &["binaryItemIDRef", "bright", "contrast", "effect", "alpha"],
        "hh:borderFill" => &["id", "threeD", "shadow", "centerLine"],
        "hh:charPr" => &[
            "id",
            "height",
            "textColor",
            "shadeColor",
            "useFontSpace",
            "useKerning",
        ],
        "hh:paraHead" => &[
            "start",
            "numFormat",
            "level",
            "align",
            "useInstWidth",
            "autoIndent",
            "widthAdjust",
            "textOffsetType",
            "textOffset",
            "charPrIDRef",
        ],
        "hh:paraPr" => &[
            "id",
            "tabPrIDRef",
            "condense",
            "fontLineHeight",
            "snapToGrid",
        ],
        "hh:breakSetting" => &[
            "breakLatinWord",
            "breakNonLatinWord",
            "widowOrphan",
            "keepWithNext",
            "keepLines",
            "pageBreakBefore",
            "lineWrap",
        ],
        "hh:style" => &[
            "id",
            "type",
            "name",
            "engName",
            "paraPrIDRef",
            "charPrIDRef",
            "nextStyleIDRef",
            "langID",
            "lockForm",
        ],
        _ => &[],
    }
}
fn xml_node(element: &Element) -> XmlNode {
    let mut node = XmlNode::new(&element.name.qualified);
    for attribute in &element.attributes {
        node.set(&attribute.name.qualified, &attribute.value);
    }
    let order = attribute_order(&node.name);
    if !order.is_empty() {
        node.attributes.sort_by_key(|(name, _)| {
            order
                .iter()
                .position(|key| *key == name)
                .unwrap_or(order.len())
        });
    }
    node.text = element.text.clone().unwrap_or_default();
    node.tail = element.tail.clone().unwrap_or_default();
    node.children = element.children.iter().map(xml_node).collect();
    node
}
fn normalize_xml(tree: &Element, name: &str) -> Result<Vec<u8>> {
    let mut namespaces = BTreeMap::new();
    for element in tree.iter() {
        namespace(&element.name, &mut namespaces);
        for attribute in &element.attributes {
            namespace(&attribute.name, &mut namespaces);
        }
    }
    let mut root = xml_node(tree);
    let mut declarations: Vec<_> = namespaces
        .into_iter()
        .map(|(prefix, uri)| {
            let key = if prefix.is_empty() {
                "xmlns".into()
            } else {
                format!("xmlns:{prefix}")
            };
            (key, uri)
        })
        .collect();
    declarations.extend(root.attributes);
    root.attributes = declarations;
    if name == "Contents/content.hpf" {
        for manifest in root
            .children
            .iter_mut()
            .filter(|node| node.name == "opf:manifest")
        {
            manifest.children.sort_by_key(|item| {
                let href = item
                    .attributes
                    .iter()
                    .find(|(name, _)| name == "href")
                    .map(|(_, href)| href.as_str())
                    .unwrap_or("");
                (
                    href.starts_with("BinData/"),
                    if href.starts_with("BinData/") {
                        number(href)
                    } else {
                        0
                    },
                )
            });
        }
    }
    encoded(&root)
}

/// Applies only to freshly generated direct mapper output, never native copies.
pub fn normalize(payload: &[u8], target: &str) -> Result<Vec<u8>> {
    let (mut parts, trees) = read_package(payload)?;
    let order = generated_order(&parts, &trees, target);
    for (name, tree) in &trees {
        if matches!(name.as_str(), "version.xml" | "META-INF/container.xml") {
            continue;
        }
        parts.insert(name.clone(), normalize_xml(tree, name)?);
    }
    write_package_ordered(&parts, &order)
}

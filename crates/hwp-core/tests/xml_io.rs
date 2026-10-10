use hwp_core::xml_io::{utf8_xml_text, xml_tree, QualifiedName, TokenKind, TokenReader, MAX_DEPTH};

#[test]
fn test_utf16_entities_are_rejected_in_all_package_parts() {
    let text = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><!DOCTYPE a [<!ENTITY x \"EXPANDED\">]><a>&x;</a>";
    for big in [false, true] {
        for bom in [false, true] {
            let mut bytes = if bom {
                if big {
                    vec![0xfe, 0xff]
                } else {
                    vec![0xff, 0xfe]
                }
            } else {
                vec![]
            };
            for value in text.encode_utf16() {
                bytes.extend(if big {
                    value.to_be_bytes()
                } else {
                    value.to_le_bytes()
                });
            }
            assert_eq!(xml_tree(&bytes).unwrap_err().code, "XML_ENCODING");
            assert_eq!(TokenReader::new(&bytes).err().unwrap().code, "XML_ENCODING");
        }
    }
}
#[test]
fn test_encoding_declaration_is_checked_beyond_regex_window() {
    let text = format!(
        "<?xml version=\"1.0\"{}encoding=\"ISO-8859-1\"?><a>text</a>",
        " ".repeat(250)
    );
    assert_eq!(xml_tree(text.as_bytes()).unwrap_err().code, "XML_ENCODING");
    let mut tokens = TokenReader::new(text.as_bytes()).unwrap();
    assert_eq!(tokens.next_token().unwrap_err().code, "XML_ENCODING");
}
#[test]
fn test_parser_rejects_dtd_but_allows_declaration_text_in_comments() {
    assert_eq!(
        xml_tree(b"<!DOCTYPE a [<!ENTITY x 'boom'>]><a>&x;</a>")
            .unwrap_err()
            .code,
        "UNSAFE_XML"
    );
    assert_eq!(
        xml_tree(b"<a><!-- <!DOCTYPE a> --></a>")
            .unwrap()
            .name
            .local,
        "a"
    );
}
#[test]
fn owned_tree_preserves_names_attributes_text_tail_and_empty_form() {
    let root = xml_tree(b"<r xmlns='urn:r' xmlns:p='urn:p' z='1' p:x='&amp;'>lead<p:t/>tail<p:t>text<![CDATA[<>]]>&#13;&#x1f600;</p:t>end</r>").unwrap();
    assert_eq!(root.name.namespace.as_deref(), Some("urn:r"));
    assert_eq!(root.attributes[0].value, "1");
    assert_eq!(root.attributes[1].name.namespace.as_deref(), Some("urn:p"));
    assert_eq!(root.attributes[1].value, "&");
    assert_eq!(root.text.as_deref(), Some("lead"));
    assert_eq!(root.children[0].tail.as_deref(), Some("tail"));
    assert!(root.children[0].self_closing);
    assert_eq!(root.children[1].text.as_deref(), Some("text<>\r😀"));
    assert_eq!(root.children[1].tail.as_deref(), Some("end"));
}
#[test]
fn lexical_tokens_keep_original_byte_spans_and_preorder_indexes() {
    let bytes = "\u{feff}<r xmlns:p='urn:p'>한<p:t x='>'>a&amp;b</p:t><p:tab/><p:tab></p:tab></r>"
        .as_bytes();
    let mut reader = TokenReader::new(bytes).unwrap();
    let mut starts = vec![];
    while let Some(token) = reader.next_token().unwrap() {
        if let TokenKind::Start {
            name,
            self_closing,
            element_index,
            ..
        } = token.kind
        {
            assert_eq!(
                &bytes[token.start..token.end],
                if name.local == "r" {
                    b"<r xmlns:p='urn:p'>".as_slice()
                } else if name.local == "t" {
                    b"<p:t x='>'>"
                } else if self_closing {
                    b"<p:tab/>"
                } else {
                    b"<p:tab>".as_slice()
                }
            );
            starts.push((element_index, self_closing));
        }
    }
    assert_eq!(starts, vec![(0, false), (1, false), (2, true), (3, false)]);
}
#[test]
fn malformed_xml_and_complexity_are_rejected() {
    for bytes in [
        b"<a><b></a>".as_slice(),
        b"<a/ ><b/>",
        b"<a/ ><a/>",
        b"<a>\x01</a>",
        b"<a>&unknown;</a>",
        b"<a x='1' x='2'/>",
        b"<p:a/>",
        b"<a>",
        b"text",
        b"<a/><b/>",
        b"<a><![CDATA[bad]]>tail]]></a>",
    ] {
        assert_eq!(xml_tree(bytes).unwrap_err().code, "BAD_XML", "{:?}", bytes);
    }
    let text = format!(
        "{}{}",
        "<a>".repeat(MAX_DEPTH + 1),
        "</a>".repeat(MAX_DEPTH + 1)
    );
    assert_eq!(xml_tree(text.as_bytes()).unwrap_err().code, "LIMIT");
    assert_eq!(utf8_xml_text(b"\xff").unwrap_err().code, "XML_ENCODING");
    assert_eq!(
        utf8_xml_text(b"<a>\0</a>").unwrap_err().code,
        "XML_ENCODING"
    );
}
#[test]
fn namespace_references_are_normalized_before_matching_names() {
    let root = xml_tree(b"<p:t xmlns:p='urn:&#x61;' p:x='v'/>").unwrap();
    assert_eq!(root.name.expanded(), "{urn:a}t");
    assert_eq!(root.attributes[0].name.expanded(), "{urn:a}x");
}
#[test]
fn invalid_names_processing_instructions_and_declarations_are_rejected() {
    for bytes in [
        b"<1/>".as_slice(),
        b"<a 1='v'/>",
        b"<?XML version='1.0'?><a/>",
        b"<?xml version='1.0' standalone='maybe'?><a/>",
        b"<?xml version='1.0' unknown='v'?><a/>",
        b"<a x='<'/>",
        b"<a:b:c xmlns:a='urn:a'/>",
        b"<?1 test?><a/>",
        b"<a>&#0;</a>",
    ] {
        assert_eq!(xml_tree(bytes).unwrap_err().code, "BAD_XML", "{bytes:?}");
    }
}
#[test]
fn reserved_namespace_references_and_many_bindings_match_python() {
    assert_eq!(
        xml_tree(b"<a xmlns:xml='http:&#x2f;&#x2f;www.w3.org/XML/1998/namespace' xml:lang='ko'/>")
            .unwrap()
            .attributes[0]
            .name
            .expanded(),
        "{http://www.w3.org/XML/1998/namespace}lang"
    );
    let mut xml = String::from("<a");
    for i in 0..129 {
        xml.push_str(&format!(" xmlns:p{i}='urn:{i}'"));
    }
    xml.push_str("/>");
    assert_eq!(xml_tree(xml.as_bytes()).unwrap().name.local, "a");
    assert_eq!(
        xml_tree(b"<a xmlns:p='urn:&amp;amp;'><p:x/></a>")
            .unwrap()
            .children[0]
            .name
            .namespace
            .as_deref(),
        Some("urn:&amp;")
    );
}

#[test]
fn matches_expanded_agrees_with_expanded() {
    let names = [
        QualifiedName {
            qualified: "a:x".into(),
            local: "x".into(),
            namespace: Some("urn:a".into()),
        },
        QualifiedName {
            qualified: "x".into(),
            local: "x".into(),
            namespace: None,
        },
        QualifiedName {
            qualified: "e:x".into(),
            local: "x".into(),
            namespace: Some(String::new()),
        },
    ];
    let candidates = [
        "{urn:a}x", "x", "{}x", "{urn:a}y", "{urn:a}", "{urn:ax", "urn:a}x", "{urn:b}x", "",
    ];
    for name in &names {
        for candidate in candidates {
            assert_eq!(
                name.matches_expanded(candidate),
                name.expanded() == candidate,
                "{name:?} vs {candidate}"
            );
        }
    }
}

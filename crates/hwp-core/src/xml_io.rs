//! UTF-8 XML trees and lexical tokens with a single restricted parser policy.
use crate::errors::{require, Result, WorkbenchError, MAX_NODES, MAX_PART};
use quick_xml::{
    events::{BytesStart, Event},
    name::{Namespace, NamespaceResolver, QName, ResolveResult},
    Reader, XmlVersion,
};
use std::collections::HashSet;

/// Maximum XML element nesting, to bound parsing and tree traversal.
pub const MAX_DEPTH: usize = 128;

/// A lexical name and its resolved namespace URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedName {
    pub qualified: String,
    pub local: String,
    pub namespace: Option<String>,
}
impl QualifiedName {
    /// Return the ElementTree expanded name, including braces around the URI.
    pub fn expanded(&self) -> String {
        match &self.namespace {
            Some(uri) => format!("{{{uri}}}{}", self.local),
            None => self.local.clone(),
        }
    }
}

/// An ordered, namespace-resolved attribute and normalized value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: QualifiedName,
    pub value: String,
}

/// An owned ElementTree-style element, with lexical empty-tag information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub name: QualifiedName,
    pub attributes: Vec<Attribute>,
    pub namespace_declarations: Vec<(String, String)>,
    pub text: Option<String>,
    pub tail: Option<String>,
    pub children: Vec<Element>,
    pub self_closing: bool,
    pub element_index: usize,
}
impl Element {
    /// Find an attribute by its ElementTree expanded name.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name.expanded() == name)
            .map(|attribute| attribute.value.as_str())
    }
    /// Visit this element and its descendants in document order.
    pub fn iter(&self) -> impl Iterator<Item = &Element> {
        let mut pending = vec![self];
        std::iter::from_fn(move || {
            let element = pending.pop()?;
            pending.extend(element.children.iter().rev());
            Some(element)
        })
    }
}

/// An XML event sufficient for namespace-aware, in-place document text edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Start {
        name: QualifiedName,
        attributes: Vec<Attribute>,
        namespace_declarations: Vec<(String, String)>,
        self_closing: bool,
        element_index: usize,
    },
    End {
        name: QualifiedName,
        element_index: usize,
    },
    Text(String),
}

/// A token's byte range in the original UTF-8 input; end is exclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

/// Check the XML byte encoding and return UTF-8 text without its optional BOM.
pub fn utf8_xml_text(payload: &[u8]) -> Result<&str> {
    require(
        payload.len() <= MAX_PART,
        "LIMIT",
        "XML part exceeds 32 MiB.",
    )?;
    require(
        !payload.starts_with(&[0xff, 0xfe])
            && !payload.starts_with(&[0xfe, 0xff])
            && !payload.contains(&0),
        "XML_ENCODING",
        "XML must be UTF-8 without NUL bytes.",
    )?;
    let text = std::str::from_utf8(payload)
        .map_err(|_| WorkbenchError::new("XML_ENCODING", "XML must be UTF-8."))?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(text))
}

fn bad_xml(error: impl std::fmt::Display) -> WorkbenchError {
    WorkbenchError::new("BAD_XML", error.to_string())
}

fn valid_char(value: char) -> bool {
    matches!(value, '\t' | '\n' | '\r')
        || matches!(value as u32, 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)
}

// XML 1.0 name ranges; names are validated separately from event tokenization.
fn name_start(c: char) -> bool {
    matches!(c, ':' | '_')
        || matches!(c as u32, 0x41..=0x5a | 0x61..=0x7a | 0xc0..=0xd6 | 0xd8..=0xf6 | 0xf8..=0x2ff | 0x370..=0x37d | 0x37f..=0x1fff | 0x200c..=0x200d | 0x2070..=0x218f | 0x2c00..=0x2fef | 0x3001..=0xd7ff | 0xf900..=0xfdcf | 0xfdf0..=0xfffd | 0x10000..=0xeffff)
}
fn name_char(c: char) -> bool {
    name_start(c)
        || matches!(c, '-' | '.')
        || matches!(c as u32, 0x30..=0x39 | 0xb7 | 0x300..=0x36f | 0x203f..=0x2040)
}
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(name_start) && chars.all(name_char)
}
fn valid_qname(name: &str) -> bool {
    let parts: Vec<_> = name.split(':').collect();
    parts.len() <= 2
        && parts
            .iter()
            .all(|part| valid_name(part) && !part.contains(':'))
}
fn validate_declaration(declaration: &str) -> Result<()> {
    let opening = BytesStart::from_content(declaration, 3);
    let mut position = 0;
    let mut standalone = false;
    for attribute in opening.attributes() {
        let attribute = attribute.map_err(bad_xml)?;
        let key = attribute.key.into_inner();
        let value = attribute.value.as_ref();
        let allowed = match key {
            "version" => {
                position == 0
                    && value.split_once('.').is_some_and(|(major, minor)| {
                        !major.is_empty()
                            && !minor.is_empty()
                            && major.bytes().all(|c| c.is_ascii_digit())
                            && minor.bytes().all(|c| c.is_ascii_digit())
                    })
            }
            "encoding" => position == 1 && !standalone,
            "standalone" => (position == 1 || position == 2) && matches!(value, "yes" | "no"),
            _ => false,
        };
        require(allowed, "BAD_XML", "Invalid XML declaration.")?;
        if key == "standalone" {
            standalone = true;
        }
        position += 1;
    }
    require(position > 0, "BAD_XML", "Invalid XML declaration.")
}

fn resolved_name(
    resolver: &NamespaceResolver,
    name: QName<'_>,
    attribute: bool,
) -> Result<QualifiedName> {
    require(
        valid_qname(name.into_inner()),
        "BAD_XML",
        "Invalid XML name.",
    )?;
    let (namespace, local) = if attribute {
        resolver.resolve_attribute(name)
    } else {
        resolver.resolve_element(name)
    };
    let namespace = match namespace {
        ResolveResult::Unbound => None,
        ResolveResult::Bound(uri) => Some(uri.into_inner().to_owned()),
        ResolveResult::Unknown(prefix) => {
            return Err(bad_xml(format!("Unbound namespace prefix: {prefix}")))
        }
    };
    Ok(QualifiedName {
        qualified: name.into_inner().to_owned(),
        local: local.into_inner().to_owned(),
        namespace,
    })
}

/// A streaming reader enforcing the same restrictions as the owned tree parser.
pub struct TokenReader<'a> {
    reader: Reader<&'a [u8]>,
    resolver: NamespaceResolver,
    bom_length: usize,
    elements: Vec<(QualifiedName, usize)>,
    nodes: usize,
    root_seen: bool,
    finished: bool,
    declaration_seen: bool,
}
impl<'a> TokenReader<'a> {
    /// Validate the bytes before constructing the restricted XML event reader.
    pub fn new(payload: &'a [u8]) -> Result<Self> {
        let text = utf8_xml_text(payload)?;
        require(
            text.chars().all(valid_char),
            "BAD_XML",
            "Invalid XML character.",
        )?;
        let mut reader = Reader::from_reader(text.as_bytes());
        let mut resolver = NamespaceResolver::default();
        // The part byte budget already bounds the number of declared bindings.
        resolver.set_max_namespace_bindings(MAX_PART);
        reader.config_mut().check_comments = true;
        Ok(Self {
            reader,
            resolver,
            bom_length: payload.len() - text.len(),
            elements: vec![],
            nodes: 0,
            root_seen: false,
            finished: false,
            declaration_seen: false,
        })
    }

    /// Read the next significant event, preserving original byte offsets.
    pub fn next_token(&mut self) -> Result<Option<Token>> {
        if self.finished {
            return Ok(None);
        }
        loop {
            let start =
                usize::try_from(self.reader.buffer_position()).map_err(bad_xml)? + self.bom_length;
            let event = self.reader.read_event().map_err(bad_xml)?;
            let end =
                usize::try_from(self.reader.buffer_position()).map_err(bad_xml)? + self.bom_length;
            let kind = match event {
                Event::DocType(_) => {
                    return Err(WorkbenchError::new(
                        "UNSAFE_XML",
                        "DTD and entity declarations are prohibited.",
                    ))
                }
                Event::Decl(declaration) => {
                    require(
                        !self.declaration_seen && start == self.bom_length && !self.root_seen,
                        "BAD_XML",
                        "Misplaced XML declaration.",
                    )?;
                    self.declaration_seen = true;
                    validate_declaration(&declaration)?;
                    if let Some(encoding) = declaration.encoding() {
                        require(
                            encoding.map_err(bad_xml)?.eq_ignore_ascii_case("utf-8"),
                            "XML_ENCODING",
                            "XML encoding must declare UTF-8.",
                        )?;
                    }
                    continue;
                }
                Event::Start(ref opening) | Event::Empty(ref opening) => {
                    let self_closing = matches!(event, Event::Empty(_));
                    require(
                        self.elements.len() < MAX_DEPTH && self.nodes < MAX_NODES * 10,
                        "LIMIT",
                        "XML complexity exceeds limits.",
                    )?;
                    if self.elements.is_empty() {
                        require(!self.root_seen, "BAD_XML", "Multiple XML root elements.")?;
                        self.root_seen = true;
                    }
                    self.resolver
                        .push(&BytesStart::new("scope"))
                        .map_err(bad_xml)?;
                    let mut unresolved_attributes = vec![];
                    let mut attributes = vec![];
                    let mut namespace_declarations = vec![];
                    let mut names = HashSet::new();
                    for attribute in opening.attributes() {
                        let attribute = attribute.map_err(bad_xml)?;
                        require(
                            !attribute.value.contains('<'),
                            "BAD_XML",
                            "Invalid XML attribute value.",
                        )?;
                        let value = attribute
                            .normalized_value(XmlVersion::Implicit1_0)
                            .map_err(bad_xml)?
                            .into_owned();
                        require(
                            value.chars().all(valid_char),
                            "BAD_XML",
                            "Invalid XML character.",
                        )?;
                        let key = attribute.key.into_inner();
                        if key == "xmlns" || key.starts_with("xmlns:") {
                            require(valid_qname(key), "BAD_XML", "Invalid XML name.")?;
                            let prefix = key.strip_prefix("xmlns:").unwrap_or("");
                            let xml_uri = "http://www.w3.org/XML/1998/namespace";
                            require(
                                prefix != "xmlns"
                                    && value != "http://www.w3.org/2000/xmlns/"
                                    && ((prefix == "xml") == (value == xml_uri))
                                    && (prefix.is_empty() || !value.is_empty()),
                                "BAD_XML",
                                "Invalid namespace binding.",
                            )?;
                            let binding = attribute
                                .key
                                .as_namespace_binding()
                                .ok_or_else(|| bad_xml("Invalid namespace binding."))?;
                            self.resolver
                                .add(binding, Namespace(&value))
                                .map_err(bad_xml)?;
                            namespace_declarations.push((key.to_owned(), value));
                        } else {
                            unresolved_attributes.push((attribute.key, value));
                        }
                    }
                    // Declarations apply to every attribute regardless of lexical order.
                    for (key, value) in unresolved_attributes {
                        let name = resolved_name(&self.resolver, key, true)?;
                        require(
                            names.insert(name.expanded()),
                            "BAD_XML",
                            "Duplicate XML attribute.",
                        )?;
                        attributes.push(Attribute { name, value });
                    }
                    let name = resolved_name(&self.resolver, opening.name(), false)?;
                    if self_closing {
                        self.resolver.pop();
                    }
                    let element_index = self.nodes;
                    self.nodes += 1;
                    if !self_closing {
                        self.elements.push((name.clone(), element_index));
                    }
                    TokenKind::Start {
                        name,
                        attributes,
                        namespace_declarations,
                        self_closing,
                        element_index,
                    }
                }
                Event::End(_) => {
                    self.resolver.pop();
                    let (name, element_index) = self
                        .elements
                        .pop()
                        .ok_or_else(|| bad_xml("Unmatched XML end tag."))?;
                    TokenKind::End {
                        name,
                        element_index,
                    }
                }
                Event::Text(text) => {
                    let value = text.xml10_content().into_owned();
                    require(
                        !value.contains("]]>"),
                        "BAD_XML",
                        "Invalid XML character data.",
                    )?;
                    if self.elements.is_empty() {
                        require(
                            value.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r')),
                            "BAD_XML",
                            "Text outside the XML root.",
                        )?;
                        continue;
                    }
                    TokenKind::Text(value)
                }
                Event::CData(text) => {
                    require(
                        !self.elements.is_empty(),
                        "BAD_XML",
                        "CDATA outside the XML root.",
                    )?;
                    TokenKind::Text(text.xml10_content().into_owned())
                }
                Event::GeneralRef(reference) => {
                    require(
                        !self.elements.is_empty(),
                        "BAD_XML",
                        "Entity outside the XML root.",
                    )?;
                    let value = quick_xml::escape::unescape(&format!("&{};", &*reference))
                        .map_err(bad_xml)?
                        .into_owned();
                    TokenKind::Text(value)
                }
                Event::Eof => {
                    require(
                        self.root_seen && self.elements.is_empty(),
                        "BAD_XML",
                        "Missing root or unclosed XML element.",
                    )?;
                    self.finished = true;
                    return Ok(None);
                }
                Event::PI(instruction) => {
                    require(
                        valid_name(instruction.target())
                            && !instruction.target().eq_ignore_ascii_case("xml"),
                        "BAD_XML",
                        "Invalid processing instruction target.",
                    )?;
                    continue;
                }
                Event::Comment(_) => continue,
            };
            if let TokenKind::Text(value) = &kind {
                require(
                    value.chars().all(valid_char),
                    "BAD_XML",
                    "Invalid XML character.",
                )?;
            }
            return Ok(Some(Token { start, end, kind }));
        }
    }
}

fn attach(element: Element, parents: &mut [Element], root: &mut Option<Element>) {
    if let Some(parent) = parents.last_mut() {
        parent.children.push(element);
    } else {
        *root = Some(element);
    }
}

/// Parse a bounded owned tree with namespace names, ordered attributes and tails.
pub fn xml_tree(payload: &[u8]) -> Result<Element> {
    let mut reader = TokenReader::new(payload)?;
    let mut parents: Vec<Element> = vec![];
    let mut root = None;
    while let Some(token) = reader.next_token()? {
        match token.kind {
            TokenKind::Start {
                name,
                attributes,
                namespace_declarations,
                self_closing,
                element_index,
            } => {
                let element = Element {
                    name,
                    attributes,
                    namespace_declarations,
                    text: None,
                    tail: None,
                    children: vec![],
                    self_closing,
                    element_index,
                };
                if self_closing {
                    attach(element, &mut parents, &mut root);
                } else {
                    parents.push(element);
                }
            }
            TokenKind::End { .. } => {
                let element = parents
                    .pop()
                    .ok_or_else(|| bad_xml("Unmatched XML end tag."))?;
                attach(element, &mut parents, &mut root);
            }
            TokenKind::Text(value) => {
                if let Some(parent) = parents.last_mut() {
                    let target = if let Some(child) = parent.children.last_mut() {
                        &mut child.tail
                    } else {
                        &mut parent.text
                    };
                    target.get_or_insert_with(String::new).push_str(&value);
                }
            }
        }
    }
    root.ok_or_else(|| bad_xml("Missing XML root."))
}

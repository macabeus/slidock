//! The XML wrapper: the tree helpers, decoding, and every limit with its reason.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

use indexmap::IndexMap;
use slidock_io::xml::{
    attributes, child_element, child_elements, elements_of, namespace_uri, qualified_name,
    text_content,
};
use slidock_io::{XmlBudget, XmlErrorReason, XmlLimits, decode_xml, parse_xml};

use XmlErrorReason::*;

const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

/// `parse` on decoded bytes with the default limits and a fresh budget: `None` when it parses.
fn reason_with(text: &str, limits: &XmlLimits, budget: &mut XmlBudget) -> Option<XmlErrorReason> {
    parse_xml(text, limits, budget)
        .err()
        .map(|error| error.reason)
}

fn reason(text: &str) -> Option<XmlErrorReason> {
    let limits = XmlLimits::default();
    reason_with(text, &limits, &mut XmlBudget::new(&limits))
}

fn reason_of_bytes(bytes: &[u8]) -> Option<XmlErrorReason> {
    match decode_xml(bytes) {
        Ok(text) => reason(&text),
        Err(error) => Some(error.reason),
    }
}

fn parse_default(text: &str) -> roxmltree::Document<'_> {
    let limits = XmlLimits::default();
    parse_xml(text, &limits, &mut XmlBudget::new(&limits)).unwrap()
}

fn map(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[test]
fn builds_a_namespaced_element_tree_with_text_and_attributes() {
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<p:sld xmlns:p=\"{P}\" xmlns:a=\"{A}\"><p:sp id=\"4\"><a:t xml:space=\"preserve\"> Hi &amp; bye &#x263A; </a:t><a:t><![CDATA[<raw>]]></a:t></p:sp></p:sld>"
    );
    let document = parse_default(&text);
    let root = document.root_element();
    assert_eq!(
        (
            qualified_name(root),
            root.tag_name().name(),
            namespace_uri(root)
        ),
        ("p:sld", "sld", P)
    );
    let shape = child_element(root, P, "sp").unwrap();
    assert_eq!(shape.attribute("id"), Some("4"));
    let texts: Vec<_> = child_elements(shape, A, "t").collect();
    let contents: Vec<String> = texts.iter().map(|text| text_content(*text)).collect();
    assert_eq!(contents, [" Hi & bye \u{263a} ", "<raw>"]);
    assert_eq!(attributes(texts[0], None)["xml:space"], "preserve");
    assert_eq!(elements_of(root).count(), 1);
    assert_eq!(
        attributes(root, None),
        map(&[("xmlns:p", P), ("xmlns:a", A)])
    );
}

#[test]
fn strips_a_utf8_byte_order_mark_and_reads_utf16le() {
    let text = decode_xml(&[0xef, 0xbb, 0xbf, b'<', b'a', b'/', b'>']).unwrap();
    assert_eq!(parse_default(&text).root_element().tag_name().name(), "a");
    let mut utf16 = vec![0xff, 0xfe];
    for unit in "<b>\u{1f600}</b>".encode_utf16() {
        utf16.extend(unit.to_le_bytes());
    }
    let text = decode_xml(&utf16).unwrap();
    let document = parse_default(&text);
    assert_eq!(document.root_element().tag_name().name(), "b");
    assert_eq!(text_content(document.root_element()), "\u{1f600}");
    // An odd last byte and an unpaired surrogate are replaced.
    assert_eq!(
        decode_xml(&[0xff, 0xfe, 0x00, 0xd8, b'x', 0, 7]).unwrap(),
        "\u{fffd}x\u{fffd}"
    );
}

#[test]
fn refuses_a_doctype_so_no_entity_is_ever_expanded() {
    let bomb = "<!DOCTYPE x [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;&a;\">]><x>&b;</x>";
    assert_eq!(reason(bomb), Some(Doctype));
    assert_eq!(
        reason("<!DOCTYPE x SYSTEM \"file:///etc/passwd\"><x/>"),
        Some(Doctype)
    );
    assert_eq!(
        reason("<?xml version=\"1.0\"?><!-- c --><!DOCTYPE x><x/>"),
        Some(Doctype)
    );
}

#[test]
fn refuses_entities_other_than_the_predefined_five_and_numeric_references() {
    assert_eq!(reason("<x>&nbsp;</x>"), Some(Malformed));
    let document = parse_default("<x>&lt;&gt;&amp;&quot;&apos;&#65;</x>");
    assert_eq!(text_content(document.root_element()), "<>&\"'A");
}

#[test]
fn caps_depth_node_count_and_attribute_size() {
    let deep = |n: usize| "<a>".repeat(n) + &"</a>".repeat(n);
    assert_eq!(reason(&deep(256)), None);
    assert_eq!(reason(&deep(257)), Some(TooDeep));
    // Far deeper than any stack could recurse.
    assert_eq!(reason(&deep(200_000)), Some(TooDeep));
    let limits = XmlLimits {
        nodes: 10,
        presentation_nodes: 100,
        attribute_bytes: 8,
        ..XmlLimits::default()
    };
    let parts = |n: usize| format!("<r>{}</r>", "<b/>".repeat(n));
    assert_eq!(
        reason_with(&parts(9), &limits, &mut XmlBudget::new(&limits)),
        None
    );
    assert_eq!(
        reason_with(&parts(10), &limits, &mut XmlBudget::new(&limits)),
        Some(TooManyNodes)
    );
    assert_eq!(
        reason_with("<r a=\"12345678\"/>", &limits, &mut XmlBudget::new(&limits)),
        None
    );
    let error = parse_xml(
        "<r a=\"123456789\"/>",
        &limits,
        &mut XmlBudget::new(&limits),
    )
    .unwrap_err();
    assert_eq!(
        (error.reason, error.message.as_str()),
        (AttributeTooLarge, "Attribute a exceeds 8 bytes")
    );
    // Values are measured after references expand, in UTF-16 code units.
    assert_eq!(
        reason_with(
            "<r a=\"&#x1F600;&#x1F600;&#x1F600;&#x1F600;\"/>",
            &limits,
            &mut XmlBudget::new(&limits)
        ),
        None
    );
    assert_eq!(
        reason_with(
            "<r a=\"&amp;&amp;&amp;&amp;&amp;&amp;&amp;&amp;&amp;\"/>",
            &limits,
            &mut XmlBudget::new(&limits)
        ),
        Some(AttributeTooLarge)
    );
}

#[test]
fn parses_a_large_part_in_linear_time() {
    let part = format!(
        "<r>{}</r>",
        "<a:b xmlns:a=\"urn:a\" x=\"1\" a:y=\"2\">t</a:b>".repeat(200_000)
    );
    let limits = XmlLimits::default();
    let document = parse_xml(&part, &limits, &mut XmlBudget::new(&limits)).unwrap();
    let first = elements_of(document.root_element()).next().unwrap();
    assert_eq!(
        attributes(first, None),
        map(&[("xmlns:a", "urn:a"), ("x", "1"), ("a:y", "2")])
    );
}

#[test]
fn counts_text_runs_as_a_streaming_parser_does() {
    let limits = XmlLimits {
        nodes: 3,
        ..XmlLimits::default()
    };
    let count = |text: &str| reason_with(text, &limits, &mut XmlBudget::new(&limits));
    // Root, one text run, one element.
    assert_eq!(count("<r>a<![CDATA[b]]>c<b/></r>"), None);
    // A comment or processing instruction inside a run does not end it.
    assert_eq!(count("<r>a<!-- c -->b<?pi x?>c<b/></r>"), None);
    // Text on both sides of an element is two runs.
    assert_eq!(count("<r>a<b/>c</r>"), Some(TooManyNodes));
    // Whitespace between elements is text.
    assert_eq!(count("<r> <b/></r>"), None);
    assert_eq!(count("<r> <b/> </r>"), Some(TooManyNodes));
    // Comments outside the root, and the declaration, are not nodes.
    assert_eq!(
        count("<?xml version=\"1.0\"?><!-- a --><r><b/><c/></r><!-- b -->"),
        None
    );
}

#[test]
fn draws_every_parts_nodes_from_one_budget_so_a_presentation_of_many_parts_cannot_pass_it() {
    let limits = XmlLimits {
        nodes: 10,
        presentation_nodes: 25,
        ..XmlLimits::default()
    };
    let mut budget = XmlBudget::new(&limits);
    let part = format!("<r>{}</r>", "<b/>".repeat(9));
    assert_eq!(reason_with(&part, &limits, &mut budget), None);
    assert_eq!(reason_with(&part, &limits, &mut budget), None);
    assert_eq!(budget.nodes, 5);
    let error = parse_xml(&part, &limits, &mut budget).unwrap_err();
    assert_eq!(
        (error.reason, error.message.as_str()),
        (
            TooManyNodes,
            "The presentation holds more than 25 XML nodes"
        )
    );
    assert_eq!(budget.nodes, -1);
}

#[test]
fn charges_a_part_that_floods_the_parser_as_one_past_its_limit() {
    let limits = XmlLimits {
        nodes: 10,
        presentation_nodes: 100,
        ..XmlLimits::default()
    };
    // Comments are not counted, but roxmltree holds them: past twice the limit and some slack the part is refused.
    let comments = |n: usize| format!("<r>{}</r>", "<!---->".repeat(n));
    let mut budget = XmlBudget::new(&limits);
    assert_eq!(reason_with(&comments(1000), &limits, &mut budget), None);
    assert_eq!(budget.nodes, 99);
    let error = parse_xml(&comments(1100), &limits, &mut budget).unwrap_err();
    assert_eq!(
        (error.reason, error.message.as_str()),
        (TooManyNodes, "More than 10 XML nodes")
    );
    assert_eq!(budget.nodes, 89);
    // With less budget left than the part's limit, the presentation's limit is the one hit.
    let mut low = XmlBudget { nodes: 4 };
    let error = parse_xml(&comments(1100), &limits, &mut low).unwrap_err();
    assert_eq!(
        error.message,
        "The presentation holds more than 100 XML nodes"
    );
    assert_eq!(low.nodes, -1);
}

#[test]
fn names_attributes_by_namespace_when_given_the_prefixes_to_use_whatever_the_writer_bound() {
    let prefixes = [("urn:draw", "draw")];
    let document = parse_default(
        "<r xmlns:d=\"urn:draw\" xmlns:x=\"urn:other\"><e d:name=\"a\" x:name=\"b\" plain=\"c\"/><e xmlns:draw=\"urn:other\" draw:name=\"d\"/></r>",
    );
    let elements: Vec<_> = elements_of(document.root_element()).collect();
    assert_eq!(
        attributes(elements[0], Some(&prefixes)),
        map(&[("draw:name", "a"), ("{urn:other}name", "b"), ("plain", "c")])
    );
    // `draw` bound to another namespace is that namespace's.
    assert_eq!(
        attributes(elements[1], Some(&prefixes)),
        map(&[("xmlns:draw", "urn:other"), ("{urn:other}name", "d")])
    );
    // Without prefixes, names are as written.
    assert_eq!(
        attributes(elements[0], None),
        map(&[("d:name", "a"), ("x:name", "b"), ("plain", "c")])
    );
    // `xml:` names its namespace like any other.
    let spaced = parse_default("<t xml:space=\"preserve\"/>");
    assert_eq!(
        attributes(spaced.root_element(), Some(&prefixes)),
        map(&[("{http://www.w3.org/XML/1998/namespace}space", "preserve")])
    );
}

#[test]
fn keeps_the_written_prefix_of_an_element() {
    let document = parse_default("<a:r xmlns:a=\"urn:x\" xmlns:b=\"urn:x\"><b:c/><a:c/></a:r>");
    let names: Vec<&str> = elements_of(document.root_element())
        .map(qualified_name)
        .collect();
    assert_eq!(names, ["b:c", "a:c"]);
}

#[test]
fn normalises_attribute_whitespace_and_keeps_text_whitespace() {
    let document = parse_default("<r a=\"x\ty\nz\">\r\n line </r>");
    let root = document.root_element();
    assert_eq!(root.attribute("a"), Some("x y z"));
    assert_eq!(text_content(root), "\n line ");
}

#[test]
fn caps_the_attributes_of_one_element_namespace_declarations_included() {
    let many = |n: usize| {
        let attributes: Vec<String> = (0..n).map(|i| format!("a{i}=\"1\"")).collect();
        format!("<r {}/>", attributes.join(" "))
    };
    let limits = XmlLimits {
        attributes: 4,
        ..XmlLimits::default()
    };
    let check = |text: &str| reason_with(text, &limits, &mut XmlBudget::new(&limits));
    assert_eq!(check(&many(4)), None);
    assert_eq!(check(&many(5)), Some(TooManyAttributes));
    assert_eq!(
        check("<r xmlns=\"urn:a\" xmlns:b=\"urn:b\" b:c=\"1\" d='2'/>"),
        None
    );
    assert_eq!(
        check("<r xmlns=\"urn:a\" xmlns:b=\"urn:b\" xmlns:c=\"urn:b\" b:c=\"1\" d='2'/>"),
        Some(TooManyAttributes)
    );
    // A redeclaration of the parent's binding still counts.
    assert_eq!(
        check("<r xmlns:b=\"urn:b\"><e xmlns:b=\"urn:b\" a=\"1\" c=\"2\" d=\"3\" f=\"4\"/></r>"),
        Some(TooManyAttributes)
    );
    // A namespace URI is measured as an attribute value.
    let short = XmlLimits {
        attribute_bytes: 4,
        ..XmlLimits::default()
    };
    let error = parse_xml(
        "<r xmlns:b=\"urn:bb\"/>",
        &short,
        &mut XmlBudget::new(&short),
    )
    .unwrap_err();
    assert_eq!(error.message, "Attribute xmlns:b exceeds 4 bytes");
}

#[test]
fn refuses_malformed_parts_undeclared_prefixes_and_utf16be() {
    assert_eq!(reason("<a><b></a>"), Some(Malformed));
    assert_eq!(reason("<p:a/>"), Some(Malformed));
    let error = parse_default_err("");
    assert_eq!(
        (error.reason, error.message.as_str()),
        (Malformed, "The part has no root element")
    );
    assert_eq!(reason_of_bytes(&[0xfe, 0xff, 0, 0x3c]), Some(Encoding));
}

fn parse_default_err(text: &str) -> slidock_io::XmlError {
    let limits = XmlLimits::default();
    parse_xml(text, &limits, &mut XmlBudget::new(&limits)).unwrap_err()
}

/// `n` nested elements, each declaring one more prefix.
fn nested_declarations(n: usize) -> String {
    let opens: String = (0..n).map(|i| format!("<a xmlns:p{i}=\"u\">")).collect();
    opens + &"</a>".repeat(n)
}

#[test]
fn refuses_namespace_scopes_that_would_cost_more_than_linear_time() {
    // roxmltree copies every namespace in scope into each element that declares one: refused before it parses.
    assert_eq!(reason(&nested_declarations(5000)), Some(TooManyNamespaces));
    // Too deep long before the namespaces cost anything.
    assert_eq!(reason(&nested_declarations(40_000)), Some(TooDeep));
    assert_eq!(reason(&nested_declarations(100)), None);
    // Every element's names are searched through the namespaces in its scope.
    let scoped = |declarations: usize, children: usize| {
        let declared: String = (0..declarations)
            .map(|i| format!(" xmlns:p{i}=\"u\""))
            .collect();
        format!("<r{declared}>{}</r>", "<p0:b/>".repeat(children))
    };
    assert_eq!(reason(&scoped(500, 100_000)), Some(TooManyNamespaces));
    assert_eq!(reason(&scoped(30, 100_000)), None);
    // An element that declares a namespace copies its parent's 1,500 in scope, however long the element is.
    let copied = |declares: &str| {
        let level = |prefix: char| -> String {
            (0..500)
                .map(|i| format!(" xmlns:{prefix}{i}=\"u\""))
                .collect()
        };
        let child = format!("<b{declares} pad=\"{}\"/>", "x".repeat(1000));
        format!(
            "<r{}><s{}><t{}>{}</t></s></r>",
            level('p'),
            level('q'),
            level('v'),
            child.repeat(1000)
        )
    };
    assert_eq!(reason(&copied("")), None);
    assert_eq!(reason(&copied(" xmlns:z=\"u\"")), Some(TooManyNamespaces));
    // A redeclared prefix is one namespace in scope.
    let redeclared = format!("{}{}", "<a xmlns:p=\"u\">".repeat(200), "</a>".repeat(200));
    assert_eq!(reason(&redeclared), None);
}

#[test]
fn refuses_more_distinct_namespaces_than_roxmltree_holds() {
    let declared = |n: usize| {
        let children: String = (0..n).map(|i| format!("<b xmlns:a=\"u{i}\"/>")).collect();
        format!("<r>{children}</r>")
    };
    assert_eq!(reason(&declared(65_535)), None);
    let error = parse_default_err(&declared(65_536));
    assert_eq!(
        (error.reason, error.message.as_str()),
        (TooManyNamespaces, "More than 65536 distinct namespaces")
    );
}

#[test]
fn refuses_an_element_of_too_many_attributes_before_parsing_them() {
    let attributes: String = (0..200_000).map(|i| format!(" a{i}=\"1\"")).collect();
    assert_eq!(
        reason(&format!("<r{attributes}/>")),
        Some(TooManyAttributes)
    );
}

#[test]
fn reports_what_comes_first_in_the_part_and_charges_the_nodes_before_a_refusal() {
    let deep = |n: usize| "<a>".repeat(n) + &"</a>".repeat(n);
    let limits = XmlLimits::default();
    let charged = |text: &str| {
        let mut budget = XmlBudget::new(&limits);
        let reason = reason_with(text, &limits, &mut budget);
        (reason, XmlBudget::new(&limits).nodes - budget.nodes)
    };
    // The root, two elements, 255 nested ones, and the one too deep.
    assert_eq!(
        charged(&format!("<r><b/><b/>{}</r>", deep(300))),
        (Some(TooDeep), 259)
    );
    // A text run before the nesting.
    assert_eq!(
        charged(&format!("<r><b/>t{}</r>", deep(300))),
        (Some(TooDeep), 259)
    );
    // The refused element is the root.
    let attributes: String = (0..600).map(|i| format!(" a{i}=\"1\"")).collect();
    assert_eq!(
        charged(&format!("<r{attributes}/>")),
        (Some(TooManyAttributes), 1)
    );
    // Depth is checked before the attributes are counted.
    assert_eq!(
        charged(&format!(
            "<r><b/>{}<a{attributes}/>{}</r>",
            "<a>".repeat(255),
            "</a>".repeat(255)
        )),
        (Some(TooDeep), 258)
    );
    // A malformation or another limit earlier in the part comes first.
    assert_eq!(
        charged(&format!("<r><a b='1' b='2'/>{}</r>", deep(300))).0,
        Some(Malformed)
    );
    let small = XmlLimits {
        nodes: 10,
        attribute_bytes: 4,
        ..XmlLimits::default()
    };
    let first = |text: &str| reason_with(text, &small, &mut XmlBudget::new(&small));
    assert_eq!(
        first(&format!("<r>{}{}</r>", "<b/>".repeat(10), deep(300))),
        Some(TooManyNodes)
    );
    assert_eq!(
        first(&format!("<r><b c='12345'/>{}</r>", deep(300))),
        Some(AttributeTooLarge)
    );
    // A malformation after the refusal does not.
    assert_eq!(reason(&format!("<r>{}</r><x/>", deep(300))), Some(TooDeep));
}

#[test]
fn refuses_character_references_to_no_character() {
    let limits = XmlLimits::default();
    let charged = |text: &str| {
        let mut budget = XmlBudget::new(&limits);
        let reason = reason_with(text, &limits, &mut budget);
        (reason, XmlBudget::new(&limits).nodes - budget.nodes)
    };
    for text in [
        "<a>&#xD800;</a>",
        "<a>&#55296;</a>",
        "<a>&#x110000;</a>",
        "<a>&#x99999999999;</a>",
    ] {
        assert_eq!(charged(text), (Some(Malformed), 1), "{text}");
    }
    let error = parse_default_err("<a>&#xDFFF;</a>");
    assert_eq!(error.message, "&#xDFFF; names no character");
    // In an attribute the element is not counted; in text, the run is not.
    assert_eq!(charged("<r><b c='&#xDFFF;'/></r>"), (Some(Malformed), 1));
    assert_eq!(charged("<r><b/>t&#xD800;</r>"), (Some(Malformed), 2));
    assert_eq!(charged("<r>x<!--c-->y&#xD800;</r>"), (Some(Malformed), 2));
    let document = parse_default("<a b='&#x10FFFF;&#xD7FF;'>&#xE000;&#65;</a>");
    let root = document.root_element();
    assert_eq!(root.attribute("b"), Some("\u{10ffff}\u{d7ff}"));
    assert_eq!(text_content(root), "\u{e000}A");
    // Not references: CDATA, comments, processing instructions.
    assert_eq!(
        reason("<a><![CDATA[&#xD800;]]><!-- &#xD800; --><?pi &#xD800;?></a>"),
        None
    );
}

#[test]
fn refuses_namespace_declarations_that_xml_namespaces_forbids() {
    for text in [
        "<a xmlns:p=''/>",
        "<a xmlns:p='&#32;'/>",
        "<a xmlns:p='u'><b xmlns:p=''/></a>",
        "<a xmlns:xmlns='u'/>",
        "<a xmlns:p=' http://www.w3.org/2000/xmlns/'/>",
        "<a xmlns=' http://www.w3.org/XML/1998/namespace '/>",
        "<a xmlns:p='http://www.w3.org/XML/1998/namespace '/>",
        "<:a/>",
        "<a: xmlns:a='u'/>",
        "<a :b='1'/>",
    ] {
        assert_eq!(reason(text), Some(Malformed), "{text}");
    }
    // The default namespace may be undeclared, and the `xml` prefix declared as itself.
    assert_eq!(reason("<a xmlns=''/>"), None);
    assert_eq!(reason("<a xmlns=' '/>"), None);
    let document = parse_default(
        "<a xmlns:xml='http://www.w3.org/XML/1998/namespace' xml:lang='en' xmlns:x='urn:x'/>",
    );
    assert_eq!(
        attributes(document.root_element(), None),
        map(&[
            ("xmlns:xml", "http://www.w3.org/XML/1998/namespace"),
            ("xml:lang", "en"),
            ("xmlns:x", "urn:x")
        ])
    );
}

#[test]
fn trims_namespace_uris() {
    let document =
        parse_default("<p:a xmlns:p=' urn:p\u{feff}' p:x='1' xmlns=' urn:d '><b/></p:a>");
    let root = document.root_element();
    assert_eq!(namespace_uri(root), "urn:p");
    assert_eq!(namespace_uri(elements_of(root).next().unwrap()), "urn:d");
    // Declarations keep the value as written.
    assert_eq!(
        attributes(root, Some(&[("urn:p", "p")])),
        map(&[
            ("xmlns:p", " urn:p\u{feff}"),
            ("p:x", "1"),
            ("xmlns", " urn:d ")
        ])
    );
    // Two attributes are one once their namespaces are trimmed.
    assert_eq!(
        reason("<a xmlns:p='u' xmlns:q=' u' p:x='1' q:x='2'/>"),
        Some(Malformed)
    );
    assert_eq!(
        reason("<a xmlns:p='u' xmlns:q=' u' p:x='1' q:y='2'/>"),
        None
    );
}

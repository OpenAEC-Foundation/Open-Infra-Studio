//! XML documents for the readers. roxmltree parses nested elements
//! recursively, so a deeply nested file overflows the stack of the thread
//! that reads it, and a stack overflow aborts the whole plugin
//! (`catch_unwind` does not catch it). [`parse`] measures the nesting
//! first and parses on a thread with enough stack, like the CityGML reader.

use roxmltree::Document;

/// The deepest element nesting of `text`, counted without parsing it.
/// roxmltree reads nested elements recursively, so this bounds the stack
/// the parse needs. Comments, CDATA, processing instructions and quoted
/// attribute values do not count. A DOCTYPE is read the way roxmltree reads
/// it (`tokenizer.rs`, `parse_doctype`). An entity in its internal subset can
/// expand to markup, entity in entity up to ten levels deep, so each `<` in
/// that subset counts ten times on top of the body's nesting.
fn nesting(text: &str) -> usize {
    let b = text.as_bytes();
    // The index just past `pat` from `at`, or the end of the text.
    let past = |at: usize, pat: &[u8]| b.get(at..).and_then(|r| r.windows(pat.len()).position(|w| w == pat)).map_or(b.len(), |p| at + p + pat.len());
    // The index just past the first `>` from `i` outside quoted values;
    // `count` gets the number of `<` inside them.
    let quoted_end = |mut i: usize, count: &mut usize| {
        let mut quote = None;
        while let Some(&c) = b.get(i) {
            i += 1;
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), b'<') => *count += 1,
                (None, b'"' | b'\'') => quote = Some(c),
                (None, b'>') => break,
                _ => {}
            }
        }
        i
    };
    let (mut i, mut depth, mut deepest, mut dtd) = (0, 0usize, 0usize, 0usize);
    while let Some(p) = b.get(i..).and_then(|r| r.iter().position(|&c| c == b'<')) {
        let at = i + p;
        let rest = &b[at..];
        if rest.starts_with(b"<!--") {
            i = past(at + 4, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            i = past(at + 9, b"]]>");
        } else if rest.starts_with(b"<?") {
            i = past(at + 2, b"?>");
        } else if rest.starts_with(b"<!") {
            // A DOCTYPE: its name and quoted identifiers up to `[` or `>`.
            let mut j = at + 2;
            let mut quote = None;
            let mut subset = false;
            while let Some(&c) = b.get(j) {
                j += 1;
                match (quote, c) {
                    (Some(q), c) if c == q => quote = None,
                    (Some(_), _) => {}
                    (None, b'"' | b'\'') => quote = Some(c),
                    (None, b'[') => { subset = true; break; }
                    (None, b'>') => break,
                    _ => {}
                }
            }
            // The internal subset, declaration by declaration. ELEMENT,
            // ATTLIST and NOTATION end at their first `>`, ENTITY at the
            // first `>` outside quotes. roxmltree rejects anything else,
            // so the rest is scanned as body.
            while subset {
                while b.get(j).is_some_and(u8::is_ascii_whitespace) {
                    j += 1;
                }
                let decl = &b[j.min(b.len())..];
                if decl.starts_with(b"<!--") {
                    j = past(j + 4, b"-->");
                } else if decl.starts_with(b"<?") {
                    j = past(j + 2, b"?>");
                } else if decl.starts_with(b"<!ENTITY") {
                    dtd += 1;
                    j = quoted_end(j + 8, &mut dtd);
                } else if decl.starts_with(b"<!ELEMENT") || decl.starts_with(b"<!ATTLIST") || decl.starts_with(b"<!NOTATION") {
                    j = past(j + 2, b">");
                } else {
                    if decl.starts_with(b"]") {
                        j = past(j + 1, b">");
                    }
                    subset = false;
                }
            }
            i = j;
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            i = quoted_end(at + 2, &mut 0);
        } else {
            // A start tag; an empty element is parsed one level down too.
            i = quoted_end(at + 1, &mut 0);
            deepest = deepest.max(depth + 1);
            if !b[..i].ends_with(b"/>") {
                depth += 1;
            }
        }
    }
    deepest.saturating_add(dtd.saturating_mul(10))
}

/// Parses `text` on a thread with enough stack for its nesting: deep but
/// valid XML still reads, and XML too deep for any stack gives an error
/// instead of a stack overflow. Errors are roxmltree's own messages.
pub fn parse(text: &str, allow_dtd: bool) -> Result<Document<'_>, String> {
    let parse = || Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd, ..Default::default() }).map_err(|e| e.to_string());
    let depth = nesting(text);
    if depth <= 64 {
        return parse();
    }
    let too_deep = || format!("{} ({depth})", crate::lang::tr("XML te diep genest", "XML nested too deeply"));
    let stack = depth.checked_mul(32 * 1024).and_then(|s| s.checked_add(2 << 20)).ok_or_else(too_deep)?;
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new().stack_size(stack).spawn_scoped(scope, parse).map_err(|_| too_deep())?;
        handle.join().map_err(|_| "XML-parser gestopt".to_string())?
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nesting_counts_elements_only() {
        assert_eq!(nesting("<a><b/><c><d></d></c></a>"), 3);
        assert_eq!(nesting("<a x='>' y=\"/>\"><b/></a>"), 2);
        assert_eq!(nesting("<?xml version=\"1.0\"?><a><!-- <b><c> --><![CDATA[<b><c>]]><?pi <b> ?></a>"), 1);
        assert_eq!(nesting("<!DOCTYPE a SYSTEM \"a.dtd\"><a><b/></a>"), 2);
        // Markup in entities can expand anywhere: each `<` in the subset counts ten times.
        assert_eq!(nesting("<!DOCTYPE a [<!-- ]> --><!ENTITY e \"<b><c/></b>\">]><a>&e;</a>"), 1 + 10 * 4);
        assert_eq!(nesting("<!DOCTYPE a [<!ENTITY x 'it''s'><?pi ]> ?>]><a><b><c/></b></a>"), 3 + 10);
        // roxmltree ends ELEMENT, ATTLIST and NOTATION at their first `>`:
        // a `<!--` or `<?` inside one starts no comment or instruction.
        assert_eq!(nesting("<!DOCTYPE r [<!ELEMENT r <!-- >]><r><a><b/></a>--></r>"), 3);
        assert_eq!(nesting("<!DOCTYPE r [<!ATTLIST r x CDATA \"<?p\" >]><r><a><b/></a>?></r>"), 3);
    }

    #[test]
    fn deep_markup_from_an_entity_gets_an_answer() {
        let e = format!("{}{}", "<a>".repeat(20_000), "</a>".repeat(20_000));
        let text = format!("<!DOCTYPE r [<!ENTITY e \"{e}\">]><r>&e;</r>");
        // A plugin command's stack; either a document or an error, no abort.
        let result = std::thread::Builder::new().stack_size(2 << 20).spawn(move || parse(&text, true).map(|d| d.descendants().count())).unwrap().join().unwrap();
        assert!(result.as_ref().map_or_else(|e| e.contains("XML"), |n| *n > 20_000), "{result:?}");
    }

    #[test]
    fn errors_keep_the_roxmltree_message() {
        let e = parse("<a><b></a>", false).unwrap_err();
        assert_eq!(e, Document::parse("<a><b></a>").unwrap_err().to_string());
    }
}

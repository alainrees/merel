//! The text inside an attachment, for the search index: PDF, Word, Excel and PowerPoint
//! (docx, xlsx, pptx), OpenDocument (odt, ods, odp), HTML, a forwarded message and plain text
//! such as csv or an e-invoice in XML.
//!
//! Attachments come from anyone, so every reader here treats its input as hostile: nothing is
//! inflated past a fixed size (a small zip or PDF can unpack to gigabytes), XML with a DTD is
//! refused (entities that expand), and a reader that panics on a broken file costs only that
//! attachment's text. What comes out is capped as well.

pub mod recognize;

use std::io::{Cursor, Read};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The most text one attachment gives the index, in bytes.
pub const MAX_TEXT: usize = 128 * 1024;
/// Attachments larger than this are left alone.
pub const MAX_INPUT: usize = 20 * 1024 * 1024;
/// The most a single compressed stream or zip entry may unpack to.
const MAX_UNPACKED: usize = 32 * 1024 * 1024;
/// The most pages of a PDF that are read.
const MAX_PAGES: usize = 500;
/// The most XML nodes one part of an Office file may hold.
const MAX_NODES: u32 = 4_000_000;

/// What kind of file an attachment is, as far as reading its text goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pdf,
    Word,
    Excel,
    PowerPoint,
    OpenDocument,
    Html,
    Xml,
    Message,
    Text,
}

/// The kind of an attachment by its name, its MIME type and its first bytes; `None` for what
/// holds no text to read (pictures, archives, old binary Office files).
pub fn kind(name: &str, mime: &str, bytes: &[u8]) -> Option<Kind> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let mime = mime.to_ascii_lowercase();
    let by_ext = match ext.as_str() {
        "pdf" => Some(Kind::Pdf),
        "docx" | "docm" | "dotx" => Some(Kind::Word),
        "xlsx" | "xlsm" | "xltx" => Some(Kind::Excel),
        "pptx" | "pptm" | "ppsx" => Some(Kind::PowerPoint),
        "odt" | "ods" | "odp" | "ott" => Some(Kind::OpenDocument),
        "html" | "htm" | "xhtml" => Some(Kind::Html),
        "xml" => Some(Kind::Xml),
        "eml" => Some(Kind::Message),
        "txt" | "text" | "csv" | "tsv" | "md" | "log" | "ics" | "vcf" | "json" | "rtf" => Some(Kind::Text),
        _ => None,
    };
    if by_ext.is_some() {
        return by_ext;
    }
    match mime.as_str() {
        "application/pdf" => Some(Kind::Pdf),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => Some(Kind::Word),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Some(Kind::Excel),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => Some(Kind::PowerPoint),
        m if m.starts_with("application/vnd.oasis.opendocument.") => Some(Kind::OpenDocument),
        "text/html" => Some(Kind::Html),
        "text/xml" | "application/xml" => Some(Kind::Xml),
        "message/rfc822" => Some(Kind::Message),
        m if m.starts_with("text/") => Some(Kind::Text),
        _ if bytes.starts_with(b"%PDF-") => Some(Kind::Pdf),
        _ => None,
    }
}

/// The text of an attachment, or `None` when there is none to read (or the file is broken,
/// encrypted, or too large).
pub fn text(name: &str, mime: &str, bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_INPUT {
        return None;
    }
    let kind = kind(name, mime, bytes)?;
    let read = || match kind {
        Kind::Pdf => pdf(bytes),
        Kind::Word => office(bytes, &|n| {
            n == "word/document.xml" || is_numbered(n, "word/header") || is_numbered(n, "word/footer") || n == "word/footnotes.xml"
        }),
        Kind::Excel => office(bytes, &|n| n == "xl/sharedStrings.xml" || is_numbered(n, "xl/worksheets/sheet")),
        Kind::PowerPoint => office(bytes, &|n| is_numbered(n, "ppt/slides/slide") || is_numbered(n, "ppt/notesSlides/notesSlide")),
        Kind::OpenDocument => office(bytes, &|n| n == "content.xml"),
        Kind::Html => Some(merel_mime::html_to_text(&decode(bytes))),
        Kind::Xml => xml_text(&decode(bytes)),
        Kind::Message => Some(merel_mime::parse_body(bytes).text),
        Kind::Text => Some(decode(bytes)),
    };
    let text = match catch_unwind(AssertUnwindSafe(read)) {
        Ok(text) => text?,
        Err(_) => {
            log::info!("attachment {name}: its text could not be read");
            return None;
        }
    };
    let text = tidy(&text);
    (!text.is_empty()).then_some(text)
}

/// Whether `name` is `stem` followed by a number and `.xml`, like `ppt/slides/slide12.xml`.
fn is_numbered(name: &str, stem: &str) -> bool {
    name.strip_prefix(stem).and_then(|rest| rest.strip_suffix(".xml")).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Bytes as text: UTF-16 when it says so, else UTF-8 with anything else replaced.
fn decode(bytes: &[u8]) -> String {
    let utf16 = |little: bool| {
        let units: Vec<u16> =
            bytes[2..].as_chunks::<2>().0.iter().map(|c| if little { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) }).collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, ..] => utf16(true),
        [0xFE, 0xFF, ..] => utf16(false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Runs of blank lines and spaces folded, control characters dropped, and cut to [`MAX_TEXT`].
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_TEXT));
    let mut blank_lines = 0;
    for line in text.lines() {
        let line: String = line.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank_lines += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank_lines > 0 { "\n\n" } else { "\n" });
        }
        blank_lines = 0;
        if out.len() + line.len() > MAX_TEXT {
            let mut end = MAX_TEXT.saturating_sub(out.len());
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            out.push_str(&line[..end]);
            break;
        }
        out.push_str(&line);
    }
    out
}

fn pdf(bytes: &[u8]) -> Option<String> {
    let options = lopdf::LoadOptions { max_decompressed_size: Some(MAX_UNPACKED), ..Default::default() };
    let doc = lopdf::Document::load_mem_with_options(bytes, options).ok()?;
    let mut out = String::new();
    for page in doc.get_pages().keys().take(MAX_PAGES) {
        // A page that cannot be read leaves the others.
        if let Ok(text) = doc.extract_text_with_limit(&[*page], MAX_UNPACKED) {
            out.push_str(&text);
            out.push('\n');
        }
        if out.len() > MAX_TEXT {
            break;
        }
    }
    Some(out)
}

/// The text of the parts of a zipped Office file that `wanted` picks, in the order of their
/// numbers (slide 2 before slide 10).
fn office(bytes: &[u8], wanted: &dyn Fn(&str) -> bool) -> Option<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let mut names: Vec<String> = archive.file_names().filter_map(Result::ok).filter(|n| wanted(n)).map(|n| n.into_owned()).collect();
    let number = |n: &str| -> u64 {
        let digits: String = n.chars().filter(char::is_ascii_digit).collect();
        digits.parse().unwrap_or(0)
    };
    names.sort_by(|a, b| {
        let stem = |n: &str| n.trim_end_matches(".xml").trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
        stem(a).cmp(&stem(b)).then_with(|| number(a).cmp(&number(b)))
    });
    // The document body before the headers, footers and notes around it.
    names.sort_by_key(|n| !(n == "word/document.xml" || n == "xl/sharedStrings.xml" || n.starts_with("ppt/slides/")));
    let mut out = String::new();
    for name in names {
        let Ok(entry) = archive.by_name(&name) else { continue };
        let mut xml = Vec::new();
        // An entry that unpacks past the limit is cut there and then fails to parse.
        if entry.take(MAX_UNPACKED as u64 + 1).read_to_end(&mut xml).is_err() || xml.len() > MAX_UNPACKED {
            continue;
        }
        if let Some(text) = xml_text(&String::from_utf8_lossy(&xml)) {
            out.push_str(&text);
            out.push_str("\n\n");
        }
        if out.len() > MAX_TEXT {
            break;
        }
    }
    Some(out)
}

/// The words of an XML document: its text, with a line break at each paragraph or row and a
/// space between cells. Field codes, deleted text, formulas and spreadsheet cells that only
/// point into the shared strings are left out.
fn xml_text(xml: &str) -> Option<String> {
    let options = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: MAX_NODES, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(xml, options).ok()?;
    let mut out = String::new();
    for node in doc.descendants() {
        if node.is_element() {
            match node.tag_name().name() {
                "p" | "h" | "tr" | "row" | "br" | "cr" | "si" => out.push('\n'),
                "tab" | "c" | "tc" | "table-cell" | "s" => out.push(' '),
                _ => {}
            }
            continue;
        }
        if !node.is_text() {
            continue;
        }
        let skipped = node.ancestors().any(|a| {
            matches!(a.tag_name().name(), "instrText" | "delText" | "f")
                || (a.tag_name().name() == "v" && a.parent().is_some_and(|c| c.attribute("t") == Some("s")))
        });
        if !skipped && let Some(t) = node.text() {
            out.push_str(t);
        }
        if out.len() > MAX_TEXT * 2 {
            break;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zipped(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in parts {
            w.start_file(*name, options).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn word_text_comes_out_in_paragraphs_without_field_codes_or_deleted_words() {
        let doc = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Fac</w:t></w:r><w:r><w:t>tuur 2026-118</w:t></w:r></w:p>
<w:p><w:r><w:instrText>PAGE</w:instrText></w:r><w:r><w:t>Totaal € 1.250,00</w:t></w:r><w:r><w:delText>weg</w:delText></w:r></w:p>
</w:body></w:document>"#;
        let footer = r#"<w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>Keukenstudio Delft</w:t></w:r></w:p></w:ftr>"#;
        let bytes = zipped(&[("word/footer1.xml", footer), ("word/document.xml", doc), ("word/styles.xml", "<x>stijl</x>")]);
        let text = text("offerte.docx", "application/octet-stream", &bytes).unwrap();
        assert_eq!(text, "Factuur 2026-118\n\nTotaal € 1.250,00\n\nKeukenstudio Delft");
    }

    #[test]
    fn excel_gives_shared_strings_and_numbers_but_not_formulas() {
        let strings = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>Artikel</t></si><si><t>Prijs</t></si></sst>"#;
        let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row><c t="s"><v>0</v></c><c t="s"><v>1</v></c></row>
<row><c t="inlineStr"><is><t>Werkblad</t></is></c><c><f>SUM(B1:B9)</f><v>499.95</v></c></row>
</sheetData></worksheet>"#;
        let bytes = zipped(&[("xl/worksheets/sheet1.xml", sheet), ("xl/sharedStrings.xml", strings)]);
        let text = text("prijzen.xlsx", "", &bytes).unwrap();
        assert!(text.contains("Artikel") && text.contains("Prijs") && text.contains("Werkblad 499.95"), "{text:?}");
        assert!(!text.contains("SUM"), "{text:?}");
        assert!(!text.split_whitespace().any(|w| w == "0" || w == "1"), "indexes into the strings stay out: {text:?}");
    }

    #[test]
    fn slides_come_in_their_order_and_opendocument_reads_too() {
        let slide = |t: &str| format!(r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:sld>"#);
        let bytes = zipped(&[("ppt/slides/slide10.xml", &slide("tien")), ("ppt/slides/slide2.xml", &slide("twee"))]);
        assert_eq!(text("plan.pptx", "", &bytes).unwrap(), "twee\n\ntien");
        let content = r#"<office:document-content xmlns:office="o" xmlns:text="t"><office:body><text:h>Notulen</text:h><text:p>Besloten:<text:s/>verbouwing</text:p></office:body></office:document-content>"#;
        let bytes = zipped(&[("content.xml", content), ("styles.xml", "<s>nee</s>")]);
        assert_eq!(text("notulen.odt", "", &bytes).unwrap(), "Notulen\nBesloten: verbouwing");
    }

    #[test]
    fn plain_text_html_and_xml_read_and_a_dtd_is_refused() {
        assert_eq!(text("lijst.csv", "", b"naam;bedrag\r\nJan;12,50\r\n").unwrap(), "naam;bedrag\nJan;12,50");
        assert_eq!(text("x", "text/plain", &[0xFF, 0xFE, b'h', 0, b'i', 0]).unwrap(), "hi");
        let html = text("pagina.html", "", b"<html><body><h1>Welkom</h1><script>evil()</script><p>bij ons</p></body></html>").unwrap();
        assert!(html.contains("Welkom") && html.contains("bij ons") && !html.contains("evil"), "{html:?}");
        assert_eq!(text("factuur.xml", "", b"<Invoice><ID>INV-7</ID><Amount>12.00</Amount></Invoice>").unwrap(), "INV-712.00");
        let bomb = br#"<?xml version="1.0"?><!DOCTYPE l [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;">]><l>&b;</l>"#;
        assert_eq!(text("bom.xml", "", bomb), None);
    }

    #[test]
    fn pictures_broken_files_and_bombs_give_nothing() {
        assert_eq!(text("foto.jpg", "image/jpeg", b"\xFF\xD8\xFF"), None);
        assert_eq!(text("oud.doc", "application/msword", b"\xD0\xCF\x11\xE0"), None);
        assert_eq!(text("kapot.docx", "", b"PK\x03\x04 not really"), None);
        assert_eq!(text("kapot.pdf", "", b"%PDF-1.4 nothing here"), None);
        assert_eq!(text("leeg.txt", "", b""), None);
        // A zip entry that unpacks to more than the limit is skipped, not read into memory.
        let huge = "<w:document xmlns:w=\"w\"><w:p><w:t>".to_string() + &"a".repeat(MAX_UNPACKED + 10) + "</w:t></w:p></w:document>";
        assert_eq!(text("bom.docx", "", &zipped(&[("word/document.xml", &huge)])), None);
    }

    #[test]
    fn long_text_is_cut_at_the_limit_on_a_character() {
        let long = "één ".repeat(MAX_TEXT);
        let text = text("lang.txt", "", long.as_bytes()).unwrap();
        assert!(text.len() <= MAX_TEXT && text.starts_with("één"));
    }

    #[test]
    fn a_pdf_gives_the_text_of_its_pages() {
        use lopdf::content::{Content, Operation};
        use lopdf::{Document, Object, Stream, dictionary};
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 24.into()]),
                Operation::new("Td", vec![100.into(), 600.into()]),
                Operation::new("Tj", vec![Object::string_literal("Ordernummer 4711 verzonden")]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => content_id });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
                "Resources" => resources_id, "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        doc.compress();
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let text = text("bevestiging.pdf", "application/pdf", &bytes).unwrap();
        assert!(text.contains("Ordernummer 4711 verzonden"), "{text:?}");
    }
}

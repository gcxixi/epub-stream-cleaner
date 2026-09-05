use epub_stream_cleaner::{clean_epub, CleanOptions};
use std::fs::File;
use std::io::{Read, Write};
use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

fn write_fixture(path: &std::path::Path, xhtml: &str) {
    let file = File::create(path).unwrap();
    let mut archive = ZipWriter::new(file);
    archive
        .start_file(
            "mimetype",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
    archive.write_all(b"application/epub+zip").unwrap();
    archive
        .start_file("META-INF/container.xml", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(
            br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
        )
        .unwrap();
    archive
        .start_file("OEBPS/content.opf", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(
            br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest><item id="content" href="content.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="content"/></spine></package>"#,
        )
        .unwrap();
    archive
        .start_file("OEBPS/content.xhtml", SimpleFileOptions::default())
        .unwrap();
    archive.write_all(xhtml.as_bytes()).unwrap();
    archive.finish().unwrap();
}

fn read_entry(path: &std::path::Path, name: &str) -> String {
    let file = File::open(path).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    let mut entry = archive.by_name(name).unwrap();
    let mut content = String::new();
    entry.read_to_string(&mut content).unwrap();
    content
}

#[test]
fn strict_xhtml_keeps_namespace_and_semantic_noteref() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.epub");
    let output = directory.path().join("output.epub");
    write_fixture(
        &input,
        r#"<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>Book</title></head><body><br/><a href="ftp://example.com" epub:type="noteref">[1]</a><a href="#note-1">note</a><a href="https://example.com">ad</a></body></html>"#,
    );

    clean_epub(&input, &output, &CleanOptions::default()).unwrap();
    let content = read_entry(&output, "OEBPS/content.xhtml");
    assert!(content.contains("xmlns=\"http://www.w3.org/1999/xhtml\""));
    assert!(content.contains("<br/>") || content.contains("<br />"));
    assert!(content.contains("ftp://example.com"));
    assert!(content.contains("epub:type=\"noteref\""));
    assert!(content.contains("href=\"#note-1\""));
    assert!(!content.contains("href=\"https://example.com\""));
}

#[test]
fn clean_is_idempotent_for_a_stable_xhtml_fixture() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.epub");
    let first = directory.path().join("first.epub");
    let second = directory.path().join("second.epub");
    write_fixture(
        &input,
        r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><body><p class="ad-banner"><img src="cover.jpg"/>Offer</p><a href="https://example.com">Chapter</a><a href="#note-1">Footnote</a></body></html>"#,
    );

    clean_epub(&input, &first, &CleanOptions::default()).unwrap();
    clean_epub(&first, &second, &CleanOptions::default()).unwrap();
    assert_eq!(
        read_entry(&first, "OEBPS/content.xhtml"),
        read_entry(&second, "OEBPS/content.xhtml")
    );
}
